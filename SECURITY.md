# Security

Lessr runs on your machine and forwards requests to the provider you configured. It stores tool output handles and receipt rows in a local SQLite file under your config directory. It does not send data anywhere else.

Packs are signed with ed25519. The engine refuses unsigned or mis-signed packs.

Report vulnerabilities to security@lessr.dev. Please do not open public issues for security reports.
