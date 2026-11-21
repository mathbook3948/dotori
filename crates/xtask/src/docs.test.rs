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
