# 문서

이 목록은 `mise run docs`로 만든다. 직접 고치지 않는다. 문서 규칙은 `docs/CLAUDE.md`에 있다.

## architecture

- [에이전트 실행](architecture/agents.md): 세션을 무엇으로 실행하는지 정한다.
- [브라우저](architecture/browser.md): 에이전트가 쓰는 브라우저를 어떻게 띄우고, 모델과 사용자가 어떻게 움직이는지 정한다.
- [여러 PC](architecture/devices.md): 여러 PC에 설치한 dotori를 비서 하나로 묶는 방법을 정한다.
- [Discord 연결](architecture/discord.md): Discord를 주인이 비서와 말하는 채널로 어떻게 쓰는지 정한다.
- [그래프 DB](architecture/graph.md): 메모리의 대상과 관계를 저장하고 질의하는 그래프 DB를 Rust로 직접 만든다.
- [LLM 연결](architecture/llm.md): 메인 세션이 LLM API와 주고받는 방식을 정한다.
- [MCP](architecture/mcp.md): 에이전트가 쓰는 도구를 어떻게 제공하는지 정한다.
- [메모리](architecture/memory.md): 비서가 몇 달에서 몇 년 동안 기억을 유지하는 방법을 정한다.
- [프로세스 구조](architecture/processes.md): dotori를 어떤 프로세스로 나눠 실행하는지 정한다.
- [루틴](architecture/routines.md): 주인이 정해 둔 시간에 비서가 알아서 하는 일을 어떻게 다루는지 정한다.
- [세션](architecture/sessions.md): 주인이 어디서 비서와 말하고, 대화를 어떤 세션으로 나누는지 정한다.
- [Slack 연결](architecture/slack.md): Slack을 채널과 연동으로 어떻게 쓰는지 정한다.
- [Telegram 연결](architecture/telegram.md): Telegram을 주인이 비서와 말하는 채널로 어떻게 쓰는지 정한다.
- [비밀값 보관](architecture/vault.md): API 키, 봇 토큰, 연동한 사이트의 계정 같은 비밀값을 어디에 두고 어떻게 다른 PC로 옮기는지 정한다.
- [창](architecture/windows.md): 채팅창과 설정창이 어떻게 보이는지 정한다.

## engineering

- [배포](engineering/release.md): dotori 실행 파일을 어떻게 만들어 사용자에게 내놓는지 정한다.
- [견고함](engineering/robustness.md): 외부 입력, 파일, 자식 프로세스, 실패를 다루는 코드가 처음부터 지킬 것을 정한다.
- [저장](engineering/storage.md): SQLite 파일의 테이블과 열을 어떻게 짓고 관리하는지 정한다.
- [테스트](engineering/testing.md): 창과 데몬을 어떻게 테스트하고, 에이전트가 화면을 어떻게 확인하는지 정한다.

## product

- [제품 방향](product/direction.md): dotori가 어떤 제품인지 정의한다.
- [개발 순서](product/roadmap.md): dotori를 어떤 순서로 만드는지 정한다.
