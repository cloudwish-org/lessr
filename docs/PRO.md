# Lessr Pro

Same binary, more crates. The free engine is complete and unlimited; Pro adds mechanisms that need data, coordination or ongoing maintenance.

| Pro mechanism | What it adds |
| --- | --- |
| cachefix | Auto-fixes the breaks the free detector reports |
| keepalive, compact | Keeps a long session's cache warm across breaks; compacts and evicts by rent only when the cache is already cold |
| mcp | Lazy-loads MCP tool schemas |
| zoom | Map-then-zoom reads: outline first, exact range on demand |
| delta | Repeated commands show only what changed |
| editverify | Post-edit re-reads return only the changed region plus a syntax check |
| batch | Routes non-interactive runs through provider batch endpoints |
| prefix | One warm prefix across subagents, bots and sessions |
| rent | Lifetime cost of every read |
| router | Cheapest model per task with a quality guard |
| full pack | 200+ rules, updated weekly |

`lessr pro` upgrades in place. https://lessr.dev/pro
