# Benchmark Results

This page contains the latest benchmark results for the Krab framework. The data is generated automatically by our CI pipeline.

## Latest Summary (2026-03-08)

**Profile:** load (Samples: 1000, Concurrency: 10)
**Service started with:** `KRAB_RATE_LIMIT_REFILL_PER_SEC=2000 KRAB_RATE_LIMIT_CAPACITY=4000`

| Route | Krab (Rust) p95 |
|---|---:|
| `home` | 6.16 ms |
| `blog_post` | 5.78 ms |
| `api_status` | 372.69 ms |
| `api_mutation` | 10.82 ms |
| `health` | 8.62 ms |

> **Note:** `api_status` latency reflects round-trips to upstream services. When upstreams are unavailable the handler waits for their TCP timeout before responding — see [issue: add per-upstream call timeout](#).

## Detailed Metrics: Krab (Rust)

| Route | p50 | p95 | p99 | Mean | Max | Error % |
|---|---:|---:|---:|---:|---:|---:|
| home | 3.9 | 6.16 | 9.85 | 4.13 | 25.2 | 0.0 |
| blog_post | 3.7 | 5.78 | 8.53 | 3.91 | 19.45 | 0.0 |
| api_status | 306.85 | 372.69 | 439.12 | 311.15 | 516.73 | 0.0 |
| api_mutation | 6.53 | 10.82 | 20.08 | 6.98 | 26.3 | 0.0 |
| health | 5.46 | 8.62 | 11.52 | 5.68 | 17.5 | 0.0 |

## Historical Trends

You can view historical performance trends in the [trend history CSV](../../plans/load_test_artifacts/trend_history.csv).

## How to Reproduce

Follow the instructions in the [Benchmark Methodology](50_benchmark_methodology.md) guide, making sure to start `service_frontend` with the rate limit env vars for the chosen profile.

```bash
KRAB_RATE_LIMIT_REFILL_PER_SEC=2000 KRAB_RATE_LIMIT_CAPACITY=4000 cargo run --release -p service_frontend
python3 scripts/nft_benchmark_runner.py --frameworks krab --profile load
```
