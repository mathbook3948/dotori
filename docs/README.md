# 문서

이 목록은 `mise run docs`로 만든다. 직접 고치지 않는다. 문서 규칙은 `docs/CLAUDE.md`에 있다.

## architecture

- [에이전트 실행](architecture/agents.md): 메인 대화와 하위 작업을 무엇으로 실행하는지 정한다.
- [그래프 DB](architecture/graph.md): 메모리의 대상과 관계를 저장하고 질의하는 그래프 DB를 Rust로 직접 만든다.
- [MCP](architecture/mcp.md): 에이전트가 쓰는 도구를 어떻게 제공하는지 정한다.
- [메모리](architecture/memory.md): 비서가 몇 달에서 몇 년 동안 기억을 유지하는 방법을 정한다.
- [프로세스 구조](architecture/processes.md): lightbot을 어떤 프로세스로 나눠 실행하는지 정한다.

## product

- [제품 방향](product/direction.md): lightbot이 어떤 제품인지 정의한다.
