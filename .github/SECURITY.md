# Security policy

dotori runs on your PC with your accounts. It holds API keys in environment variables, keeps your conversations and browser logins in its data folder, runs shell commands you allow, and links your PCs to each other. A security bug in dotori can expose all of that, so please report one privately.

## Supported versions

dotori has no stable release yet. Only the latest release and the `main` branch get security fixes.

## Report a vulnerability

Do not open a public issue for a vulnerability.

Report it through [GitHub private vulnerability reporting](https://github.com/mathbook3948/dotori/security/advisories/new). Include:

- the version or commit you tested,
- the operating system,
- the steps that show the problem, and
- what an attacker can do with it.

The maintainer replies within 7 days. After a fix is released, the advisory is published with credit to you, unless you ask to stay anonymous.

## In scope

- A person other than the owner who makes dotori act for them, for example through a Slack, Telegram, or Discord message, a web page, or a tool result.
- A call that acts toward other people, runs a shell command, or writes a file without the owner's confirmation.
- A leak of API keys, tokens, the raw record, notes, or browser cookies.
- A weakness in the link between the owner's PCs.

## Out of scope

- Attacks that need control of the owner's user account on the PC.
- What a model does with a tool after the owner allows the call.
