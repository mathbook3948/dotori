# dotori

Rust로 만드는 가벼운 개인 비서.

문서는 `docs/README.md`에서 찾는다.

## 작업 방식

- 간단하지 않은 작업은 `.claude/skills/rigor-mode/SKILL.md`를 읽고 그 절차를 따른다.
- 커밋 전에 `mise run quick`을 통과시킨다.
- `mise run quick`에는 창 테스트와 라이선스 검사가 빠져 있다.
- 창 코드를 바꿨으면 `mise run window-test`도 통과시킨다.
- 전체 검사인 `mise run check`는 CI가 돌린다. 로컬에서 따로 돌리지 않는다.
- worktree끼리 `CARGO_TARGET_DIR`를 같이 쓰지 않는다. cargo는 수정 시각으로만 판단해서 다른 브랜치의 빌드 결과를 그대로 쓴다.
- 창의 모양과 움직임을 바꿀 때는 `.claude/skills/gpui-design/SKILL.md` 절차를 따른다. 창이 옮겨 갈 디자인은 루트의 `DESIGN.md`에 있다.
