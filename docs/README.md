# 문서

이 목록은 `mise run docs`로 만든다. 직접 고치지 않는다. 문서 규칙은 `docs/CLAUDE.md`에 있다.

## architecture

- [에이전트 실행](architecture/agents.md): 세션을 무엇으로 실행하는지 정한다.
- [브라우저](architecture/browser.md): 에이전트가 쓰는 브라우저를 어떻게 띄우고, 모델과 사용자가 어떻게 움직이는지 정한다.
- [여러 PC](architecture/devices.md): 여러 PC에 설치한 dotori를 비서 하나로 묶는 방법을 정한다.
- [그래프 DB](architecture/graph.md): 메모리의 대상과 관계를 저장하고 질의하는 그래프 DB를 Rust로 직접 만든다.
- [LLM 연결](architecture/llm.md): 메인 세션과 방 세션이 LLM API와 주고받는 방식을 정한다.
- [MCP](architecture/mcp.md): 에이전트가 쓰는 도구를 어떻게 제공하는지 정한다.
- [메모리](architecture/memory.md): 비서가 몇 달에서 몇 년 동안 기억을 유지하는 방법을 정한다.
- [프로세스 구조](architecture/processes.md): dotori를 어떤 프로세스로 나눠 실행하는지 정한다.
- [세션](architecture/sessions.md): 대화를 어떤 세션으로 나누고, 각 세션이 어떤 기억을 볼 수 있는지 정한다.
- [Slack 연결](architecture/slack.md): Slack 채널이 무엇을 듣고 어디에 답하는지 정한다.

## product

- [제품 방향](product/direction.md): dotori가 어떤 제품인지 정의한다.
- [개발 순서](product/roadmap.md): dotori를 어떤 순서로 만드는지 정한다.
