# dotori

Rust로 만드는 가벼운 개인 비서.

문서는 `docs/README.md`에서 찾는다.

## 작업 방식

- 간단하지 않은 작업은 `.claude/skills/rigor-mode/SKILL.md`를 읽고 그 절차를 따른다.
- 외부 입력, 파일, 자식 프로세스, 실패를 다루는 코드는 구현하면서 `docs/engineering/robustness.md`로 점검한다.
- 커밋 전에 `mise run check`를 통과시킨다.
