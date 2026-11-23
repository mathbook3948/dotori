use super::*;

fn tokens(source: &str) -> Vec<Token> {
    lex(source)
        .expect("lexed")
        .into_iter()
        .map(|lexed| lexed.token)
        .collect()
}

#[test]
fn a_division_and_a_regular_expression_are_told_apart() {
    assert_eq!(
        tokens("a / 2"),
        vec![
            Token::Name(String::from("a")),
            Token::Punct("/"),
            Token::Number(2.0),
            Token::End
        ]
    );
    assert_eq!(
        tokens("f(/sign [in]/i)")[2],
        Token::Regex {
            source: String::from("sign [in]"),
            flags: String::from("i")
        }
    );
}

#[test]
fn strings_templates_and_comments_are_read() {
    assert_eq!(
        tokens("'a\\'b' // gone\n\"\\u{d55c}\\n\" /* gone */"),
        vec![
            Token::Str(String::from("a'b")),
            Token::Str(String::from("한\n")),
            Token::End
        ]
    );
    assert_eq!(
        tokens("`hi ${name + `x`}!`")[0],
        Token::Template(vec![
            Piece::Text(String::from("hi ")),
            Piece::Hole(String::from("name + `x`")),
            Piece::Text(String::from("!")),
        ])
    );
}

#[test]
fn the_longest_operator_wins_and_lines_are_counted() {
    assert_eq!(tokens("a?.b ?? c === d")[1], Token::Punct("?."));
    assert_eq!(tokens("a?.b ?? c === d")[3], Token::Punct("??"));
    assert_eq!(tokens("a?.b ?? c === d")[5], Token::Punct("==="));
    let lexed = lex("a\n\nb").expect("lexed");
    assert_eq!(lexed[1].line, 3);
    assert!(lex("'open").expect_err("unclosed").contains("line 1"));
    assert!(lex("a # b").is_err());
}
