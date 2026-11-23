//! Runs a parsed script. Values are JavaScript's, objects the browser owns are `Native` and are
//! handled by the host, and every call is awaited as it is made, so `await` only passes values on.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use serde_json::Value as Json;
use serde_json::json;

use crate::script::parse::Body;
use crate::script::parse::Expr;
use crate::script::parse::Function;
use crate::script::parse::Item;
use crate::script::parse::Key;
use crate::script::parse::Pattern;
use crate::script::parse::Stmt;

pub(crate) type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

const STEPS: usize = 200_000;
const LOGS: usize = 200;
const SHOWN: usize = 4000;

#[derive(Clone)]
pub(crate) enum Value {
    Undefined,
    Null,
    Bool(bool),
    Number(f64),
    Str(Arc<str>),
    Array(Arc<Mutex<Vec<Value>>>),
    Object(Arc<Mutex<Vec<(String, Value)>>>),
    Function(Arc<Closure>),
    Regex(Arc<(String, String)>),
    Native(Native),
    Global(&'static str),
    Method(Arc<(Value, String)>),
}

impl std::fmt::Debug for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&shown(self))
    }
}

/// Something only the host knows how to use, such as the page or a locator. `data` is the host's.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Native {
    pub(crate) kind: &'static str,
    pub(crate) data: Json,
}

pub(crate) struct Closure {
    function: Arc<Function>,
    scope: Arc<Scope>,
}

/// The browser side of a script: what calls on `page`, locators and the like do.
pub(crate) trait Host: Send + Sync {
    fn call<'a>(
        &'a self,
        this: &'a Native,
        method: &'a str,
        args: Vec<Value>,
    ) -> BoxFuture<'a, Result<Value, String>>;

    fn property(&self, this: &Native, name: &str) -> Option<Value>;
}

/// What a script printed and what it returned.
#[derive(Debug, PartialEq)]
pub(crate) struct Outcome {
    pub(crate) logs: Vec<String>,
    pub(crate) value: Option<String>,
}

enum Flow {
    Normal,
    Return(Value),
    Break,
    Continue,
}

/// How a script stops early. A thrown value can be caught; running out of steps cannot.
pub(crate) enum Stop {
    Thrown(Value),
    Fatal(String),
}

type Done<T> = Result<T, Stop>;

#[derive(Default)]
struct Scope {
    names: Mutex<HashMap<String, (Value, bool)>>,
    parent: Option<Arc<Scope>>,
}

impl Scope {
    fn child(parent: &Arc<Scope>) -> Arc<Scope> {
        Arc::new(Scope {
            names: Mutex::new(HashMap::new()),
            parent: Some(Arc::clone(parent)),
        })
    }

    fn find(&self, name: &str) -> Option<Value> {
        if let Some((value, _)) = self.names.lock().expect("unpoisoned").get(name) {
            return Some(value.clone());
        }
        self.parent.as_ref().and_then(|parent| parent.find(name))
    }

    fn declare(&self, name: &str, value: Value, constant: bool) {
        self.names
            .lock()
            .expect("unpoisoned")
            .insert(name.to_owned(), (value, constant));
    }

    fn set(&self, name: &str, value: Value) -> Result<(), String> {
        if let Some(slot) = self.names.lock().expect("unpoisoned").get_mut(name) {
            if slot.1 {
                return Err(format!("`{name}` is a constant"));
            }
            slot.0 = value;
            return Ok(());
        }
        match &self.parent {
            Some(parent) => parent.set(name, value),
            None => Err(format!("`{name}` is not declared")),
        }
    }
}

/// Runs `body` with `page` as the page, and returns what it printed and returned.
pub(crate) async fn run(body: &[Stmt], host: &dyn Host, page: Native) -> Result<Outcome, String> {
    let global = Arc::new(Scope::default());
    global.declare("page", Value::Native(page), true);
    for name in [
        "console",
        "Math",
        "JSON",
        "Object",
        "Array",
        "Promise",
        "String",
        "Number",
        "Boolean",
        "Date",
        "parseInt",
        "parseFloat",
        "isNaN",
        "RegExp",
        "Error",
    ] {
        global.declare(name, Value::Global(name), true);
    }
    let machine = Machine {
        host,
        logs: Mutex::new(Vec::new()),
        steps: AtomicUsize::new(0),
    };
    let flow = machine.block(body, &global).await;
    let logs = machine.logs.into_inner().expect("unpoisoned");
    match flow {
        Ok(Flow::Return(value)) if !matches!(value, Value::Undefined) => Ok(Outcome {
            logs,
            value: Some(cut(&shown(&value))),
        }),
        Ok(_) => Ok(Outcome { logs, value: None }),
        Err(Stop::Thrown(value)) => {
            let mut said = logs.join("\n");
            if !said.is_empty() {
                said.push_str("\n\n");
            }
            Err(format!("{said}Uncaught {}", message_of(&value)))
        }
        Err(Stop::Fatal(error)) => Err(error),
    }
}

struct Machine<'h> {
    host: &'h dyn Host,
    logs: Mutex<Vec<String>>,
    steps: AtomicUsize,
}

fn throw<T>(message: impl Into<String>) -> Done<T> {
    Err(Stop::Thrown(error_value(&message.into())))
}

fn error_value(message: &str) -> Value {
    object(vec![
        (String::from("name"), Value::Str(Arc::from("Error"))),
        (String::from("message"), Value::Str(Arc::from(message))),
    ])
}

fn object(fields: Vec<(String, Value)>) -> Value {
    Value::Object(Arc::new(Mutex::new(fields)))
}

fn array(items: Vec<Value>) -> Value {
    Value::Array(Arc::new(Mutex::new(items)))
}

fn text(value: &str) -> Value {
    Value::Str(Arc::from(value))
}

impl Machine<'_> {
    fn step(&self) -> Done<()> {
        if self.steps.fetch_add(1, Ordering::Relaxed) > STEPS {
            return Err(Stop::Fatal(String::from(
                "the script ran too many steps; is a loop endless?",
            )));
        }
        Ok(())
    }

    fn block<'a>(&'a self, body: &'a [Stmt], scope: &'a Arc<Scope>) -> BoxFuture<'a, Done<Flow>> {
        Box::pin(async move {
            for stmt in body {
                if let Stmt::Function { name, function } = stmt {
                    scope.declare(name, closure(function, scope), false);
                }
            }
            for stmt in body {
                match self.statement(stmt, scope).await? {
                    Flow::Normal => {}
                    other => return Ok(other),
                }
            }
            Ok(Flow::Normal)
        })
    }

    fn statement<'a>(&'a self, stmt: &'a Stmt, scope: &'a Arc<Scope>) -> BoxFuture<'a, Done<Flow>> {
        Box::pin(async move {
            self.step()?;
            match stmt {
                Stmt::Empty | Stmt::Function { .. } => Ok(Flow::Normal),
                Stmt::Expr(expr) => {
                    self.eval(expr, scope).await?;
                    Ok(Flow::Normal)
                }
                Stmt::Let {
                    names,
                    value,
                    constant,
                } => {
                    let value = match value {
                        Some(value) => self.eval(value, scope).await?,
                        None => Value::Undefined,
                    };
                    bind(names, value, scope, *constant)?;
                    Ok(Flow::Normal)
                }
                Stmt::If {
                    test,
                    then,
                    otherwise,
                } => {
                    if truthy(&self.eval(test, scope).await?) {
                        self.statement(then, &Scope::child(scope)).await
                    } else if let Some(otherwise) = otherwise {
                        self.statement(otherwise, &Scope::child(scope)).await
                    } else {
                        Ok(Flow::Normal)
                    }
                }
                Stmt::Block(body) => self.block(body, &Scope::child(scope)).await,
                Stmt::Return(value) => Ok(Flow::Return(match value {
                    Some(value) => self.eval(value, scope).await?,
                    None => Value::Undefined,
                })),
                Stmt::Break => Ok(Flow::Break),
                Stmt::Continue => Ok(Flow::Continue),
                Stmt::Throw(value) => Err(Stop::Thrown(self.eval(value, scope).await?)),
                Stmt::ForOf {
                    name,
                    constant,
                    items,
                    body,
                } => {
                    let items = self.eval(items, scope).await?;
                    for item in iterate(&items)? {
                        self.step()?;
                        let inner = Scope::child(scope);
                        bind(name, item, &inner, *constant)?;
                        match self.statement(body, &inner).await? {
                            Flow::Break => break,
                            Flow::Return(value) => return Ok(Flow::Return(value)),
                            Flow::Normal | Flow::Continue => {}
                        }
                    }
                    Ok(Flow::Normal)
                }
                Stmt::For {
                    init,
                    test,
                    update,
                    body,
                } => {
                    let outer = Scope::child(scope);
                    if let Some(init) = init {
                        self.statement(init, &outer).await?;
                    }
                    loop {
                        self.step()?;
                        if let Some(test) = test
                            && !truthy(&self.eval(test, &outer).await?)
                        {
                            break;
                        }
                        match self.statement(body, &Scope::child(&outer)).await? {
                            Flow::Break => break,
                            Flow::Return(value) => return Ok(Flow::Return(value)),
                            Flow::Normal | Flow::Continue => {}
                        }
                        if let Some(update) = update {
                            self.eval(update, &outer).await?;
                        }
                    }
                    Ok(Flow::Normal)
                }
                Stmt::While { test, body } => {
                    while truthy(&self.eval(test, scope).await?) {
                        self.step()?;
                        match self.statement(body, &Scope::child(scope)).await? {
                            Flow::Break => break,
                            Flow::Return(value) => return Ok(Flow::Return(value)),
                            Flow::Normal | Flow::Continue => {}
                        }
                    }
                    Ok(Flow::Normal)
                }
                Stmt::Try {
                    body,
                    name,
                    catch,
                    finally,
                } => {
                    let mut outcome = self.block(body, &Scope::child(scope)).await;
                    if let (Err(Stop::Thrown(thrown)), Some(catch)) = (&outcome, catch) {
                        let inner = Scope::child(scope);
                        if let Some(name) = name {
                            inner.declare(name, thrown.clone(), false);
                        }
                        outcome = self.block(catch, &inner).await;
                    }
                    if let Some(finally) = finally {
                        match self.block(finally, &Scope::child(scope)).await? {
                            Flow::Normal => {}
                            other => return Ok(other),
                        }
                    }
                    outcome
                }
            }
        })
    }

    fn eval<'a>(&'a self, expr: &'a Expr, scope: &'a Arc<Scope>) -> BoxFuture<'a, Done<Value>> {
        Box::pin(async move {
            self.step()?;
            Ok(match expr {
                Expr::Number(n) => Value::Number(*n),
                Expr::Str(s) => text(s),
                Expr::Bool(b) => Value::Bool(*b),
                Expr::Null => Value::Null,
                Expr::Undefined => Value::Undefined,
                Expr::Regex { source, flags } => {
                    Value::Regex(Arc::new((source.clone(), flags.clone())))
                }
                Expr::Template(parts) => {
                    let mut joined = String::new();
                    for part in parts {
                        joined.push_str(&string(&self.eval(part, scope).await?));
                    }
                    text(&joined)
                }
                Expr::Name(name) => match scope.find(name) {
                    Some(value) => value,
                    None => return throw(format!("{name} is not defined")),
                },
                Expr::Array(items) => array(self.items(items, scope).await?),
                Expr::Object(fields) => {
                    let mut made: Vec<(String, Value)> = Vec::new();
                    for (key, value) in fields {
                        let value = self.eval(value, scope).await?;
                        match key {
                            Key::Spread => {
                                if let Value::Object(fields) = value {
                                    for (key, value) in fields.lock().expect("unpoisoned").iter() {
                                        put(&mut made, key, value.clone());
                                    }
                                }
                            }
                            Key::Name(name) => put(&mut made, name, value),
                            Key::Computed(key) => {
                                let key = string(&self.eval(key, scope).await?);
                                put(&mut made, &key, value);
                            }
                        }
                    }
                    object(made)
                }
                Expr::Member {
                    object,
                    property,
                    optional,
                } => {
                    let target = self.eval(object, scope).await?;
                    if *optional && matches!(target, Value::Undefined | Value::Null) {
                        return Ok(Value::Undefined);
                    }
                    let key = self.key(property, scope).await?;
                    self.get(&target, &key)?
                }
                Expr::Call {
                    callee,
                    args,
                    optional,
                } => {
                    let args = self.items(args, scope).await?;
                    if let Expr::Member {
                        object,
                        property,
                        optional: member_optional,
                    } = &**callee
                    {
                        let target = self.eval(object, scope).await?;
                        if *member_optional && matches!(target, Value::Undefined | Value::Null) {
                            return Ok(Value::Undefined);
                        }
                        let key = self.key(property, scope).await?;
                        return self.call_method(&target, &key, args).await;
                    }
                    let function = self.eval(callee, scope).await?;
                    if *optional && matches!(function, Value::Undefined | Value::Null) {
                        return Ok(Value::Undefined);
                    }
                    self.call(&function, args).await?
                }
                Expr::New { callee, args } => {
                    let args = self.items(args, scope).await?;
                    match self.eval(callee, scope).await? {
                        Value::Global("Error") => {
                            error_value(&args.first().map(string).unwrap_or_default())
                        }
                        Value::Global("Date") => {
                            if args.is_empty() {
                                Value::Number(now_ms())
                            } else {
                                return throw("only `new Date()` is supported");
                            }
                        }
                        Value::Global("RegExp") => regexp(&args)?,
                        _ => {
                            return throw(
                                "only `new Error`, `new Date()` and `new RegExp` are supported",
                            );
                        }
                    }
                }
                Expr::Unary { op, value } => {
                    let value = self.eval(value, scope).await?;
                    match *op {
                        "!" => Value::Bool(!truthy(&value)),
                        "-" => Value::Number(-number(&value)),
                        "+" => Value::Number(number(&value)),
                        "typeof" => text(type_of(&value)),
                        _ => Value::Undefined,
                    }
                }
                Expr::Binary { op, left, right } => {
                    let left = self.eval(left, scope).await?;
                    let right = self.eval(right, scope).await?;
                    binary(op, &left, &right)?
                }
                Expr::Logical { op, left, right } => {
                    let left = self.eval(left, scope).await?;
                    let take_left = match *op {
                        "&&" => !truthy(&left),
                        "||" => truthy(&left),
                        _ => !matches!(left, Value::Undefined | Value::Null),
                    };
                    if take_left {
                        left
                    } else {
                        self.eval(right, scope).await?
                    }
                }
                Expr::Conditional {
                    test,
                    then,
                    otherwise,
                } => {
                    if truthy(&self.eval(test, scope).await?) {
                        self.eval(then, scope).await?
                    } else {
                        self.eval(otherwise, scope).await?
                    }
                }
                Expr::Assign { op, target, value } => {
                    let new = if *op == "=" {
                        self.eval(value, scope).await?
                    } else {
                        let old = self.eval(target, scope).await?;
                        match *op {
                            "&&=" if !truthy(&old) => return Ok(old),
                            "||=" if truthy(&old) => return Ok(old),
                            "??=" if !matches!(old, Value::Undefined | Value::Null) => {
                                return Ok(old);
                            }
                            "&&=" | "||=" | "??=" => self.eval(value, scope).await?,
                            _ => {
                                binary(&op[..op.len() - 1], &old, &self.eval(value, scope).await?)?
                            }
                        }
                    };
                    self.assign(target, new.clone(), scope).await?;
                    new
                }
                Expr::Update { op, prefix, target } => {
                    let old = number(&self.eval(target, scope).await?);
                    let new = if *op == "++" { old + 1.0 } else { old - 1.0 };
                    self.assign(target, Value::Number(new), scope).await?;
                    Value::Number(if *prefix { new } else { old })
                }
                Expr::Function(function) => closure(function, scope),
                Expr::Await(value) => self.eval(value, scope).await?,
            })
        })
    }

    async fn items(&self, items: &[Item], scope: &Arc<Scope>) -> Done<Vec<Value>> {
        let mut values = Vec::new();
        for item in items {
            match item {
                Item::One(expr) => values.push(self.eval(expr, scope).await?),
                Item::Spread(expr) => values.extend(iterate(&self.eval(expr, scope).await?)?),
            }
        }
        Ok(values)
    }

    async fn key(&self, key: &Key, scope: &Arc<Scope>) -> Done<String> {
        Ok(match key {
            Key::Name(name) => name.clone(),
            Key::Computed(expr) => string(&self.eval(expr, scope).await?),
            Key::Spread => String::new(),
        })
    }

    async fn assign(&self, target: &Expr, value: Value, scope: &Arc<Scope>) -> Done<()> {
        match target {
            Expr::Name(name) => scope.set(name, value).or_else(throw),
            Expr::Member {
                object, property, ..
            } => {
                let target = self.eval(object, scope).await?;
                let key = self.key(property, scope).await?;
                match target {
                    Value::Object(fields) => {
                        put(&mut fields.lock().expect("unpoisoned"), &key, value);
                        Ok(())
                    }
                    Value::Array(items) => {
                        let Some(index) = index_of(&key) else {
                            return throw(format!("arrays take numbers as keys, not `{key}`"));
                        };
                        let mut items = items.lock().expect("unpoisoned");
                        if index > items.len() + 10_000 {
                            return throw("that index is too far past the end");
                        }
                        if index >= items.len() {
                            items.resize(index + 1, Value::Undefined);
                        }
                        items[index] = value;
                        Ok(())
                    }
                    _ => throw(format!("cannot set `{key}` on {}", type_of(&target))),
                }
            }
            _ => throw("this cannot be assigned to"),
        }
    }

    fn get(&self, target: &Value, key: &str) -> Done<Value> {
        Ok(match target {
            Value::Undefined | Value::Null => {
                return throw(format!("cannot read `{key}` of {}", shown(target)));
            }
            Value::Str(s) => match key {
                "length" => Value::Number(s.encode_utf16().count() as f64),
                _ => match index_of(key) {
                    Some(index) => s
                        .chars()
                        .nth(index)
                        .map_or(Value::Undefined, |c| text(&c.to_string())),
                    None => Value::Method(Arc::new((target.clone(), key.to_owned()))),
                },
            },
            Value::Array(items) => match key {
                "length" => Value::Number(items.lock().expect("unpoisoned").len() as f64),
                _ => match index_of(key) {
                    Some(index) => items
                        .lock()
                        .expect("unpoisoned")
                        .get(index)
                        .cloned()
                        .unwrap_or(Value::Undefined),
                    None => Value::Method(Arc::new((target.clone(), key.to_owned()))),
                },
            },
            Value::Object(fields) => fields
                .lock()
                .expect("unpoisoned")
                .iter()
                .find(|(name, _)| name == key)
                .map_or(Value::Undefined, |(_, value)| value.clone()),
            Value::Native(native) => match self.host.property(native, key) {
                Some(value) => value,
                None => Value::Method(Arc::new((target.clone(), key.to_owned()))),
            },
            Value::Global("Math") => match key {
                "PI" => Value::Number(std::f64::consts::PI),
                _ => Value::Method(Arc::new((target.clone(), key.to_owned()))),
            },
            Value::Regex(regex) => match key {
                "source" => text(&regex.0),
                "flags" => text(&regex.1),
                _ => Value::Undefined,
            },
            _ => Value::Method(Arc::new((target.clone(), key.to_owned()))),
        })
    }

    fn call<'a>(&'a self, function: &'a Value, args: Vec<Value>) -> BoxFuture<'a, Done<Value>> {
        Box::pin(async move {
            match function {
                Value::Function(closure) => self.invoke(closure, args).await,
                Value::Method(bound) => self.call_method(&bound.0, &bound.1, args).await,
                Value::Global(name) => self.global_call(name, "", args).await,
                other => throw(format!("{} is not a function", shown(other))),
            }
        })
    }

    async fn invoke(&self, closure: &Closure, args: Vec<Value>) -> Done<Value> {
        let scope = Scope::child(&closure.scope);
        for (index, param) in closure.function.params.iter().enumerate() {
            bind(
                param,
                args.get(index).cloned().unwrap_or(Value::Undefined),
                &scope,
                false,
            )?;
        }
        match &closure.function.body {
            Body::Expr(expr) => self.eval(expr, &scope).await,
            Body::Block(body) => match self.block(body, &scope).await? {
                Flow::Return(value) => Ok(value),
                _ => Ok(Value::Undefined),
            },
        }
    }

    fn call_method<'a>(
        &'a self,
        target: &'a Value,
        key: &'a str,
        args: Vec<Value>,
    ) -> BoxFuture<'a, Done<Value>> {
        Box::pin(async move {
            match target {
                Value::Native(native) => match self.host.property(native, key) {
                    Some(function) if !matches!(function, Value::Method(_)) => {
                        self.call(&function, args).await
                    }
                    _ => self
                        .host
                        .call(native, key, args)
                        .await
                        .map_err(|error| Stop::Thrown(error_value(&error))),
                },
                Value::Str(s) => string_method(s, key, &args),
                Value::Array(items) => self.array_method(items, key, args).await,
                Value::Global(name) => self.global_call(name, key, args).await,
                Value::Object(_) => {
                    let function = self.get(target, key)?;
                    match function {
                        Value::Undefined => throw(format!("`{key}` is not a function")),
                        function => self.call(&function, args).await,
                    }
                }
                Value::Number(n) => match key {
                    "toFixed" => {
                        let digits = args.first().map_or(0.0, number).clamp(0.0, 20.0) as usize;
                        Ok(text(&format!("{n:.digits$}")))
                    }
                    "toString" => Ok(text(&number_text(*n))),
                    _ => throw(format!("numbers have no `{key}`")),
                },
                Value::Regex(regex) => match key {
                    "test" => {
                        let subject = args.first().map(string).unwrap_or_default();
                        Ok(Value::Bool(regex_test(&regex.0, &regex.1, &subject)?))
                    }
                    _ => throw(format!(
                        "regular expressions here only support `test`, not `{key}`"
                    )),
                },
                other => throw(format!("cannot call `{key}` on {}", type_of(other))),
            }
        })
    }

    async fn array_method(
        &self,
        items: &Arc<Mutex<Vec<Value>>>,
        key: &str,
        args: Vec<Value>,
    ) -> Done<Value> {
        let snapshot = || items.lock().expect("unpoisoned").clone();
        let callback = args.first().cloned().unwrap_or(Value::Undefined);
        Ok(match key {
            "push" => {
                let mut held = items.lock().expect("unpoisoned");
                held.extend(args);
                Value::Number(held.len() as f64)
            }
            "pop" => items
                .lock()
                .expect("unpoisoned")
                .pop()
                .unwrap_or(Value::Undefined),
            "shift" => {
                let mut held = items.lock().expect("unpoisoned");
                if held.is_empty() {
                    Value::Undefined
                } else {
                    held.remove(0)
                }
            }
            "join" => {
                let joiner = args.first().map_or_else(|| String::from(","), string);
                text(
                    &snapshot()
                        .iter()
                        .map(|item| match item {
                            Value::Undefined | Value::Null => String::new(),
                            other => string(other),
                        })
                        .collect::<Vec<_>>()
                        .join(&joiner),
                )
            }
            "includes" => {
                let wanted = args.first().cloned().unwrap_or(Value::Undefined);
                Value::Bool(snapshot().iter().any(|item| strict_equal(item, &wanted)))
            }
            "indexOf" => {
                let wanted = args.first().cloned().unwrap_or(Value::Undefined);
                Value::Number(
                    snapshot()
                        .iter()
                        .position(|item| strict_equal(item, &wanted))
                        .map_or(-1.0, |i| i as f64),
                )
            }
            "slice" => {
                let held = snapshot();
                let (start, end) = range(held.len(), args.first(), args.get(1));
                array(held[start..end.max(start)].to_vec())
            }
            "concat" => {
                let mut held = snapshot();
                for arg in args {
                    match arg {
                        Value::Array(more) => held.extend(more.lock().expect("unpoisoned").clone()),
                        other => held.push(other),
                    }
                }
                array(held)
            }
            "reverse" => {
                let mut held = items.lock().expect("unpoisoned");
                held.reverse();
                Value::Array(Arc::clone(items))
            }
            "sort" => {
                let mut held = snapshot();
                if matches!(callback, Value::Undefined) {
                    held.sort_by_key(string);
                } else {
                    let mut sorted: Vec<Value> = Vec::new();
                    for item in held {
                        let mut at = sorted.len();
                        for (index, existing) in sorted.iter().enumerate() {
                            if number(
                                &self
                                    .call(&callback, vec![item.clone(), existing.clone()])
                                    .await?,
                            ) < 0.0
                            {
                                at = index;
                                break;
                            }
                        }
                        sorted.insert(at, item);
                    }
                    held = sorted;
                }
                *items.lock().expect("unpoisoned") = held;
                Value::Array(Arc::clone(items))
            }
            "map" | "filter" | "forEach" | "find" | "findIndex" | "some" | "every" | "flatMap" => {
                let held = snapshot();
                let mut kept = Vec::new();
                for (index, item) in held.into_iter().enumerate() {
                    let said = self
                        .call(&callback, vec![item.clone(), Value::Number(index as f64)])
                        .await?;
                    match key {
                        "map" => kept.push(said),
                        "flatMap" => match said {
                            Value::Array(more) => {
                                kept.extend(more.lock().expect("unpoisoned").clone())
                            }
                            other => kept.push(other),
                        },
                        "filter" if truthy(&said) => kept.push(item),
                        "find" if truthy(&said) => return Ok(item),
                        "findIndex" if truthy(&said) => return Ok(Value::Number(index as f64)),
                        "some" if truthy(&said) => return Ok(Value::Bool(true)),
                        "every" if !truthy(&said) => return Ok(Value::Bool(false)),
                        _ => {}
                    }
                }
                match key {
                    "map" | "filter" | "flatMap" => array(kept),
                    "find" => Value::Undefined,
                    "findIndex" => Value::Number(-1.0),
                    "some" => Value::Bool(false),
                    "every" => Value::Bool(true),
                    _ => Value::Undefined,
                }
            }
            "reduce" => {
                let mut held = snapshot().into_iter();
                let mut total = match args.get(1) {
                    Some(start) => start.clone(),
                    None => match held.next() {
                        Some(first) => first,
                        None => return throw("reduce of an empty array with no start value"),
                    },
                };
                for item in held {
                    total = self.call(&callback, vec![total, item]).await?;
                }
                total
            }
            "flat" => {
                let mut flat = Vec::new();
                for item in snapshot() {
                    match item {
                        Value::Array(more) => flat.extend(more.lock().expect("unpoisoned").clone()),
                        other => flat.push(other),
                    }
                }
                array(flat)
            }
            _ => return throw(format!("arrays here have no `{key}`")),
        })
    }

    async fn global_call(&self, name: &str, key: &str, args: Vec<Value>) -> Done<Value> {
        let first = args.first().cloned().unwrap_or(Value::Undefined);
        Ok(match (name, key) {
            ("console", "log" | "info" | "warn" | "error" | "debug") => {
                let line = args
                    .iter()
                    .map(|arg| match arg {
                        Value::Str(s) => s.to_string(),
                        other => shown(other),
                    })
                    .collect::<Vec<_>>()
                    .join(" ");
                let mut logs = self.logs.lock().expect("unpoisoned");
                if logs.len() < LOGS {
                    logs.push(cut(&line));
                }
                Value::Undefined
            }
            ("Math", "max" | "min") => {
                let numbers = args.iter().map(number);
                Value::Number(if key == "max" {
                    numbers.fold(f64::NEG_INFINITY, f64::max)
                } else {
                    numbers.fold(f64::INFINITY, f64::min)
                })
            }
            ("Math", "round") => Value::Number((number(&first) + 0.5).floor()),
            ("Math", "floor") => Value::Number(number(&first).floor()),
            ("Math", "ceil") => Value::Number(number(&first).ceil()),
            ("Math", "abs") => Value::Number(number(&first).abs()),
            ("Math", "random") => Value::Number(f64::from(now_ms() as u32 % 1000) / 1000.0),
            ("JSON", "stringify") => {
                let pretty = args
                    .get(2)
                    .is_some_and(|space| !matches!(space, Value::Undefined | Value::Null));
                let json = to_json(&first);
                text(&if pretty {
                    serde_json::to_string_pretty(&json).unwrap_or_default()
                } else {
                    json.to_string()
                })
            }
            ("JSON", "parse") => match serde_json::from_str::<Json>(&string(&first)) {
                Ok(json) => from_json(&json),
                Err(error) => return throw(format!("JSON.parse: {error}")),
            },
            ("Object", "keys" | "values" | "entries") => match first {
                Value::Object(fields) => {
                    let fields = fields.lock().expect("unpoisoned").clone();
                    array(
                        fields
                            .into_iter()
                            .map(|(name, value)| match key {
                                "keys" => text(&name),
                                "values" => value,
                                _ => array(vec![text(&name), value]),
                            })
                            .collect(),
                    )
                }
                Value::Array(items) => {
                    let items = items.lock().expect("unpoisoned").clone();
                    array(
                        items
                            .into_iter()
                            .enumerate()
                            .map(|(index, value)| match key {
                                "keys" => text(&index.to_string()),
                                "values" => value,
                                _ => array(vec![text(&index.to_string()), value]),
                            })
                            .collect(),
                    )
                }
                _ => array(Vec::new()),
            },
            ("Object", "assign") => {
                let Value::Object(target) = &first else {
                    return throw("Object.assign needs an object");
                };
                for more in &args[1..] {
                    if let Value::Object(fields) = more {
                        let fields = fields.lock().expect("unpoisoned").clone();
                        let mut held = target.lock().expect("unpoisoned");
                        for (name, value) in fields {
                            put(&mut held, &name, value);
                        }
                    }
                }
                first
            }
            ("Array", "isArray") => Value::Bool(matches!(first, Value::Array(_))),
            ("Array", "from") => array(iterate(&first)?),
            ("Promise", "all" | "allSettled") => first,
            ("Promise", "resolve") => first,
            ("String", "") => text(&string(&first)),
            ("Number" | "parseFloat", "") => Value::Number(number(&first)),
            ("parseInt", "") => {
                let digits: String = string(&first)
                    .trim()
                    .chars()
                    .take_while(|c| c.is_ascii_digit() || *c == '-')
                    .collect();
                Value::Number(digits.parse::<f64>().unwrap_or(f64::NAN))
            }
            ("isNaN", "") => Value::Bool(number(&first).is_nan()),
            ("Boolean", "") => Value::Bool(truthy(&first)),
            ("Date", "now") => Value::Number(now_ms()),
            ("Error", "") => error_value(&string(&first)),
            ("RegExp", "") => regexp(&args)?,
            _ => {
                let shown_name = if key.is_empty() {
                    name.to_owned()
                } else {
                    format!("{name}.{key}")
                };
                return throw(format!("`{shown_name}` is not supported here"));
            }
        })
    }
}

fn closure(function: &Arc<Function>, scope: &Arc<Scope>) -> Value {
    Value::Function(Arc::new(Closure {
        function: Arc::clone(function),
        scope: Arc::clone(scope),
    }))
}

fn bind(pattern: &Pattern, value: Value, scope: &Arc<Scope>, constant: bool) -> Done<()> {
    match pattern {
        Pattern::Name(name) => scope.declare(name, value, constant),
        Pattern::Array(names) => {
            let items = iterate(&value)?;
            for (index, name) in names.iter().enumerate() {
                if let Some(name) = name {
                    scope.declare(
                        name,
                        items.get(index).cloned().unwrap_or(Value::Undefined),
                        constant,
                    );
                }
            }
        }
        Pattern::Object(names) => {
            let Value::Object(fields) = &value else {
                return throw(format!("cannot take fields out of {}", type_of(&value)));
            };
            let fields = fields.lock().expect("unpoisoned").clone();
            for (key, local) in names {
                let found = fields
                    .iter()
                    .find(|(name, _)| name == key)
                    .map_or(Value::Undefined, |(_, value)| value.clone());
                scope.declare(local, found, constant);
            }
        }
    }
    Ok(())
}

fn iterate(value: &Value) -> Done<Vec<Value>> {
    match value {
        Value::Array(items) => Ok(items.lock().expect("unpoisoned").clone()),
        Value::Str(s) => Ok(s.chars().map(|c| text(&c.to_string())).collect()),
        other => throw(format!("{} cannot be looped over", type_of(other))),
    }
}

fn put(fields: &mut Vec<(String, Value)>, key: &str, value: Value) {
    match fields.iter_mut().find(|(name, _)| name == key) {
        Some(slot) => slot.1 = value,
        None => fields.push((key.to_owned(), value)),
    }
}

fn index_of(key: &str) -> Option<usize> {
    if key.is_empty() || (key.len() > 1 && key.starts_with('0')) {
        return None;
    }
    key.parse().ok()
}

fn range(len: usize, start: Option<&Value>, end: Option<&Value>) -> (usize, usize) {
    let at = |value: Option<&Value>, default: usize| -> usize {
        match value {
            None | Some(Value::Undefined) => default,
            Some(value) => {
                let n = number(value).trunc();
                if n < 0.0 {
                    len.saturating_sub((-n) as usize)
                } else {
                    (n as usize).min(len)
                }
            }
        }
    };
    (at(start, 0), at(end, len))
}

fn string_method(s: &str, key: &str, args: &[Value]) -> Done<Value> {
    let arg = |i: usize| args.get(i).map(string).unwrap_or_default();
    Ok(match key {
        "trim" => text(s.trim()),
        "trimStart" => text(s.trim_start()),
        "trimEnd" => text(s.trim_end()),
        "toLowerCase" => text(&s.to_lowercase()),
        "toUpperCase" => text(&s.to_uppercase()),
        "includes" => Value::Bool(s.contains(&arg(0))),
        "startsWith" => Value::Bool(s.starts_with(&arg(0))),
        "endsWith" => Value::Bool(s.ends_with(&arg(0))),
        "indexOf" => Value::Number(
            s.find(&arg(0))
                .map_or(-1.0, |byte| s[..byte].chars().count() as f64),
        ),
        "split" => match args.first() {
            None | Some(Value::Undefined) => array(vec![text(s)]),
            Some(_) => {
                let by = arg(0);
                if by.is_empty() {
                    array(s.chars().map(|c| text(&c.to_string())).collect())
                } else {
                    array(s.split(&by).map(text).collect())
                }
            }
        },
        "slice" | "substring" => {
            let chars: Vec<char> = s.chars().collect();
            let (start, end) = range(chars.len(), args.first(), args.get(1));
            text(&chars[start..end.max(start)].iter().collect::<String>())
        }
        "replace" | "replaceAll" => match args.first() {
            Some(Value::Regex(_)) => {
                return throw("replace with a regular expression is not supported; use a string");
            }
            _ => {
                if key == "replace" {
                    text(&s.replacen(&arg(0), &arg(1), 1))
                } else {
                    text(&s.replace(&arg(0), &arg(1)))
                }
            }
        },
        "padStart" | "padEnd" => {
            let width = args.first().map_or(0.0, number).clamp(0.0, 1000.0) as usize;
            let fill = if args.len() > 1 {
                arg(1)
            } else {
                String::from(" ")
            };
            let have = s.chars().count();
            if have >= width || fill.is_empty() {
                text(s)
            } else {
                let pad: String = fill.chars().cycle().take(width - have).collect();
                text(&if key == "padStart" {
                    format!("{pad}{s}")
                } else {
                    format!("{s}{pad}")
                })
            }
        }
        "repeat" => text(&s.repeat(args.first().map_or(0.0, number).clamp(0.0, 1000.0) as usize)),
        "charAt" => text(
            &s.chars()
                .nth(args.first().map_or(0.0, number) as usize)
                .map(String::from)
                .unwrap_or_default(),
        ),
        "toString" => text(s),
        _ => return throw(format!("strings here have no `{key}`")),
    })
}

fn regexp(args: &[Value]) -> Done<Value> {
    match args.first() {
        Some(Value::Regex(regex)) => Ok(Value::Regex(Arc::clone(regex))),
        Some(other) => Ok(Value::Regex(Arc::new((
            string(other),
            args.get(1).map(string).unwrap_or_default(),
        )))),
        None => throw("RegExp needs a pattern"),
    }
}

fn regex_test(source: &str, flags: &str, subject: &str) -> Done<bool> {
    let fold = flags.contains('i');
    let plain = source
        .chars()
        .all(|c| c.is_alphanumeric() || c == ' ' || c == '-' || c == '_');
    if !plain {
        return throw(
            "only plain words can be tested here; test it in the page with page.evaluate",
        );
    }
    Ok(if fold {
        subject.to_lowercase().contains(&source.to_lowercase())
    } else {
        subject.contains(source)
    })
}

fn binary(op: &str, left: &Value, right: &Value) -> Done<Value> {
    Ok(match op {
        "+" => match (left, right) {
            (Value::Str(_), _)
            | (_, Value::Str(_))
            | (Value::Array(_) | Value::Object(_), _)
            | (_, Value::Array(_) | Value::Object(_)) => {
                text(&format!("{}{}", string(left), string(right)))
            }
            _ => Value::Number(number(left) + number(right)),
        },
        "-" => Value::Number(number(left) - number(right)),
        "*" => Value::Number(number(left) * number(right)),
        "/" => Value::Number(number(left) / number(right)),
        "%" => Value::Number(number(left) % number(right)),
        "**" => Value::Number(number(left).powf(number(right))),
        "===" => Value::Bool(strict_equal(left, right)),
        "!==" => Value::Bool(!strict_equal(left, right)),
        "==" => Value::Bool(loose_equal(left, right)),
        "!=" => Value::Bool(!loose_equal(left, right)),
        "<" | ">" | "<=" | ">=" => {
            let ordered = match (left, right) {
                (Value::Str(a), Value::Str(b)) => a.partial_cmp(b),
                _ => number(left).partial_cmp(&number(right)),
            };
            Value::Bool(match ordered {
                None => false,
                Some(order) => match op {
                    "<" => order.is_lt(),
                    ">" => order.is_gt(),
                    "<=" => order.is_le(),
                    _ => order.is_ge(),
                },
            })
        }
        "in" => match right {
            Value::Object(fields) => {
                let key = string(left);
                Value::Bool(
                    fields
                        .lock()
                        .expect("unpoisoned")
                        .iter()
                        .any(|(name, _)| *name == key),
                )
            }
            _ => return throw("`in` needs an object on its right"),
        },
        _ => return throw(format!("`{op}` is not supported")),
    })
}

fn strict_equal(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Undefined, Value::Undefined) | (Value::Null, Value::Null) => true,
        (Value::Bool(a), Value::Bool(b)) => a == b,
        (Value::Number(a), Value::Number(b)) => a == b,
        (Value::Str(a), Value::Str(b)) => a == b,
        (Value::Array(a), Value::Array(b)) => Arc::ptr_eq(a, b),
        (Value::Object(a), Value::Object(b)) => Arc::ptr_eq(a, b),
        (Value::Function(a), Value::Function(b)) => Arc::ptr_eq(a, b),
        (Value::Native(a), Value::Native(b)) => a == b,
        (Value::Global(a), Value::Global(b)) => a == b,
        _ => false,
    }
}

fn loose_equal(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Undefined | Value::Null, Value::Undefined | Value::Null) => true,
        (Value::Undefined | Value::Null, _) | (_, Value::Undefined | Value::Null) => false,
        (
            Value::Number(_) | Value::Str(_) | Value::Bool(_),
            Value::Number(_) | Value::Str(_) | Value::Bool(_),
        ) => {
            if let (Value::Str(a), Value::Str(b)) = (left, right) {
                a == b
            } else {
                number(left) == number(right)
            }
        }
        _ => strict_equal(left, right),
    }
}

pub(crate) fn truthy(value: &Value) -> bool {
    match value {
        Value::Undefined | Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => *n != 0.0 && !n.is_nan(),
        Value::Str(s) => !s.is_empty(),
        _ => true,
    }
}

pub(crate) fn number(value: &Value) -> f64 {
    match value {
        Value::Undefined => f64::NAN,
        Value::Null => 0.0,
        Value::Bool(b) => f64::from(u8::from(*b)),
        Value::Number(n) => *n,
        Value::Str(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                0.0
            } else {
                trimmed.parse().unwrap_or(f64::NAN)
            }
        }
        _ => f64::NAN,
    }
}

pub(crate) fn string(value: &Value) -> String {
    match value {
        Value::Undefined => String::from("undefined"),
        Value::Null => String::from("null"),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => number_text(*n),
        Value::Str(s) => s.to_string(),
        Value::Array(items) => items
            .lock()
            .expect("unpoisoned")
            .iter()
            .map(|item| match item {
                Value::Undefined | Value::Null => String::new(),
                other => string(other),
            })
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => match message_field(value) {
            Some(message) => format!("Error: {message}"),
            None => String::from("[object Object]"),
        },
        Value::Function(closure) => closure.function.source.clone(),
        Value::Regex(regex) => format!("/{}/{}", regex.0, regex.1),
        Value::Native(native) => format!("[{}]", native.kind),
        Value::Global(name) => format!("[{name}]"),
        Value::Method(bound) => format!("[function {}]", bound.1),
    }
}

pub(crate) fn number_text(n: f64) -> String {
    if n.is_nan() {
        return String::from("NaN");
    }
    if n.is_infinite() {
        return String::from(if n > 0.0 { "Infinity" } else { "-Infinity" });
    }
    if n == n.trunc() && n.abs() < 1e21 {
        return format!("{}", n as i128);
    }
    let mut shown = String::new();
    let _ = write!(shown, "{n}");
    shown
}

fn type_of(value: &Value) -> &'static str {
    match value {
        Value::Undefined => "undefined",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::Str(_) => "string",
        Value::Function(_) | Value::Method(_) | Value::Global(_) => "function",
        _ => "object",
    }
}

fn message_field(value: &Value) -> Option<String> {
    let Value::Object(fields) = value else {
        return None;
    };
    let fields = fields.lock().expect("unpoisoned");
    let is_error = fields.iter().any(|(name, value)| {
        name == "name" && matches!(value, Value::Str(s) if s.ends_with("Error"))
    });
    if !is_error {
        return None;
    }
    fields
        .iter()
        .find(|(name, _)| name == "message")
        .map(|(_, value)| string(value))
}

fn message_of(value: &Value) -> String {
    match message_field(value) {
        Some(message) => format!("Error: {message}"),
        None => shown(value),
    }
}

/// How a value reads in logs and results: strings quoted inside arrays and objects, the rest as
/// JSON where it can be.
pub(crate) fn shown(value: &Value) -> String {
    match value {
        Value::Str(s) => s.to_string(),
        Value::Array(_) | Value::Object(_) => {
            if let Some(message) = message_field(value) {
                return format!("Error: {message}");
            }
            to_json(value).to_string()
        }
        other => string(other),
    }
}

fn cut(text: &str) -> String {
    if text.chars().count() <= SHOWN {
        return text.to_owned();
    }
    let kept: String = text.chars().take(SHOWN).collect();
    format!("{kept}... ({} characters in all)", text.chars().count())
}

/// The value as JSON for the page or the result. Functions become their source text.
pub(crate) fn to_json(value: &Value) -> Json {
    fn inner(value: &Value, depth: usize) -> Json {
        if depth > 32 {
            return Json::Null;
        }
        match value {
            Value::Undefined | Value::Null => Json::Null,
            Value::Bool(b) => json!(b),
            Value::Number(n) if n.is_finite() => {
                if *n == n.trunc() && n.abs() < 9e15 {
                    json!(*n as i64)
                } else {
                    json!(n)
                }
            }
            Value::Number(_) => Json::Null,
            Value::Str(s) => json!(s.as_ref()),
            Value::Array(items) => Json::Array(
                items
                    .lock()
                    .expect("unpoisoned")
                    .iter()
                    .map(|item| inner(item, depth + 1))
                    .collect(),
            ),
            Value::Object(fields) => Json::Object(
                fields
                    .lock()
                    .expect("unpoisoned")
                    .iter()
                    .filter(|(_, value)| !matches!(value, Value::Undefined))
                    .map(|(name, value)| (name.clone(), inner(value, depth + 1)))
                    .collect(),
            ),
            other => json!(string(other)),
        }
    }
    inner(value, 0)
}

pub(crate) fn from_json(json: &Json) -> Value {
    match json {
        Json::Null => Value::Null,
        Json::Bool(b) => Value::Bool(*b),
        Json::Number(n) => Value::Number(n.as_f64().unwrap_or(f64::NAN)),
        Json::String(s) => text(s),
        Json::Array(items) => array(items.iter().map(from_json).collect()),
        Json::Object(fields) => object(
            fields
                .iter()
                .map(|(name, value)| (name.clone(), from_json(value)))
                .collect(),
        ),
    }
}

/// The source of a function passed to the host, such as to `page.evaluate`.
pub(crate) fn function_source(value: &Value) -> Option<String> {
    match value {
        Value::Function(closure) => Some(closure.function.source.clone()),
        _ => None,
    }
}

fn now_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |since| since.as_millis() as f64)
}

#[cfg(test)]
#[path = "run.test.rs"]
mod tests;
