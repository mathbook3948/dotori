use super::*;

const VALID: &str = "# 메모리\n\n기억을 유지하는 방법을 정한다.\n\n## 배경\n\n이유.\n\n## 내용\n\n- 결정.\n\n## 미정\n\n없음\n";

#[test]
fn parses_title_summary_and_domain_of_a_valid_doc() {
    assert_eq!(
        parse_doc("architecture/memory.md", VALID),
        Ok(Doc {
            domain: "architecture".into(),
            path: "architecture/memory.md".into(),
            title: "메모리".into(),
            summary: "기억을 유지하는 방법을 정한다.".into(),
        })
    );
}

#[test]
fn reports_a_missing_section() {
    let text = VALID.replace("## 미정\n\n없음\n", "");

    assert_eq!(
        parse_doc("architecture/memory.md", &text),
        Err(vec![
            "sections must be exactly ## 배경, ## 내용, ## 미정".into()
        ])
    );
}

#[test]
fn reports_a_path_outside_the_domain_layout() {
    assert_eq!(
        parse_doc("Memory.md", VALID),
        Err(vec![
            "path must be <domain>/<title>.md in lowercase letters and hyphens".into()
        ])
    );
}

#[test]
fn index_groups_docs_by_domain_in_path_order() {
    let doc = |domain: &str, name: &str| Doc {
        domain: domain.into(),
        path: format!("{domain}/{name}.md"),
        title: name.into(),
        summary: format!("{name} 요약."),
    };

    let index = render_index(&[
        doc("product", "direction"),
        doc("architecture", "mcp"),
        doc("architecture", "agents"),
    ]);

    assert!(index.ends_with(
        "\n## architecture\n\n- [agents](architecture/agents.md): agents 요약.\n- [mcp](architecture/mcp.md): mcp 요약.\n\n## product\n\n- [direction](product/direction.md): direction 요약.\n"
    ));
}

#[test]
fn reports_a_decision_line_with_two_sentences() {
    let text = VALID.replace(
        "- 결정.\n",
        "- 결정.\n- 번호는 늘기만 한다. 지운 번호는 다시 쓰지 않는다.\n",
    );

    assert_eq!(
        parse_doc("architecture/memory.md", &text),
        Err(vec![
            "line 12: a decision line holds one decision in one sentence".into()
        ])
    );
}

#[test]
fn reports_reasoning_in_a_decision_line() {
    let lines = [
        "달은 따로 센다. 길이가 달라서다.",
        "달은 길이가 다르기 때문에 따로 센다.",
        "번호를 다시 쓰지 않으므로 겹치지 않는다.",
        "겹치지 않기 위해 번호를 늘린다.",
    ];

    for line in lines {
        let text = VALID.replace("- 결정.\n", &format!("- {line}\n"));
        let problems = parse_doc("architecture/memory.md", &text).unwrap_err();
        assert!(
            problems.contains(
                &"line 11: a decision line leaves its reasoning to the commit message".into()
            ),
            "{line}"
        );
    }
}

#[test]
fn accepts_periods_inside_code_spans_and_link_targets() {
    let text = VALID.replace(
        "- 결정.\n",
        "- `a. b`는 값을 읽고 [문서](docs/a. b.md) 링크를 건다.\n",
    );

    assert!(parse_doc("architecture/memory.md", &text).is_ok());
}

#[test]
fn leaves_background_and_open_questions_alone() {
    let text = VALID
        .replace("이유.\n", "이유 하나. 이유 둘 때문이다.\n")
        .replace("없음\n", "정한다. 아직 모른다.\n");

    assert!(parse_doc("architecture/memory.md", &text).is_ok());
}

#[test]
fn accepts_a_label_and_its_list_in_one_line() {
    let text = VALID.replace("- 결정.\n", "- 1단계: core. 값 타입, 런타임, 저장 포트.\n");

    assert!(parse_doc("architecture/memory.md", &text).is_ok());
}
