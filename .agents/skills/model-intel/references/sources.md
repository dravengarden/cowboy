# Source registry

Re-verify terms and endpoints at each refresh; they change. "Reference only"
means: keep values in the local state directory, commit only URL, date and
digest, and publish derived scores rather than the source's numbers.

| Source | Measures | Effort variants | Access | Rights |
| --- | --- | --- | --- | --- |
| Artificial Analysis | intelligence index and sub-indexes, price, speed, tokens and cost to run the index | separate rows per effort | Data API (`x-api-key`, free tier) | reference only; attribution, no redistribution |
| Epoch AI benchmarking hub, ECI | many benchmarks re-run independently; capability index with intervals | some runs labelled | CSV/zip downloads, `epochai` package | CC-BY for Epoch-run data; external data keeps its original licence |
| Scale SEAL (SWE-bench Pro, HLE) | agentic SWE and expert QA with standardized harness | some rows | leaderboard pages | check page terms; vendor-reported Pro scores differ from SEAL runs |
| Terminal-Bench 2.x | agentic terminal tasks | agent+model rows | leaderboard (GitHub) | check repository licence; harness-dependent, near saturation |
| ARC Prize (ARC-AGI-2/3) | novel reasoning with cost | explicit effort rows | leaderboard pages | check terms; note harness (standard vs provider adapter) |
| ReviewBench, SWE-PRBench | code-review findings quality | per paper/run | GitHub / arXiv | per repository licence |
| LMArena (text, WebDev/Code, agent) | human preference Elo | as named by vendor | Hugging Face dataset | `preference` axis only |
| OpenRouter rankings | token share | no | Data API | `adoption` axis only; price-sensitive traffic bias |
| Stack Overflow developer survey | tool and vendor usage | no | annual results | `adoption` axis only |
| Cowboy managed-call ledger and pilots | real review outcomes, tokens, minutes | exact preset | Controller database (read-only), `cowboy call` | internal |

Excluded until revived: Aider Polyglot (no current models; reported scoring
exploit). Treat BenchLM, BenchLeader and similar aggregators as discovery only.
