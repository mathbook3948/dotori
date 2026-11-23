use super::*;

#[test]
fn a_typical_playwright_script_parses() {
    let body = parse(
        "await page.goto('https://example.com');\n\
         const button = page.getByRole('button', { name: /sign in/i });\n\
         await button.click();\n\
         for (const row of await page.locator('tr').all()) { console.log(await row.textContent()); }\n\
         const titles = await page.$$eval('h2', els => els.map(e => e.textContent));\n\
         return { title: await page.title(), count: titles.length };",
    )
    .expect("parsed");
    assert_eq!(body.len(), 6);
    assert!(matches!(body[5], Stmt::Return(Some(Expr::Object(_)))));
}

#[test]
fn an_arrow_function_keeps_its_source_for_the_page() {
    let body = parse("page.evaluate((x) => { return document.title + x; }, 1)").expect("parsed");
    let Stmt::Expr(Expr::Call { args, .. }) = &body[0] else {
        panic!("a call");
    };
    let Item::One(Expr::Function(function)) = &args[0] else {
        panic!("a function");
    };
    assert_eq!(function.source, "(x) => { return document.title + x; }");
}

#[test]
fn unsupported_syntax_is_named() {
    for (source, named) in [
        ("class A {}", "`class`"),
        ("for (const k in o) {}", "`for in`"),
        ("switch (x) {}", "`switch`"),
        ("this.x", "`this`"),
        ("let a = 1, b = 2", "one name"),
    ] {
        let error = parse(source).expect_err(source);
        assert!(error.contains(named), "{source}: {error}");
    }
}

#[test]
fn loops_try_and_destructuring_parse() {
    parse(
        "for (let i = 0; i < 3; i++) { if (i === 1) continue; }\n\
         while (false) {}\n\
         try { throw new Error('x'); } catch (e) { console.log(e.message); } finally {}\n\
         const [a, , b] = [1, 2, 3];\n\
         const { x, y: z } = { x: 1, y: 2 };\n\
         let n = 0; n += 2; n **= 2;\n\
         const s = `${a}-${b}`;",
    )
    .expect("parsed");
}
