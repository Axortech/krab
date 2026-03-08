# Benchmark Methodology

This document outlines the methodology used to benchmark the Krab framework against other full-stack web frameworks. The goal is to provide fair, reproducible, and transparent performance comparisons for common web application patterns.

## Scope

### Frameworks Tested

We benchmark Krab against a representative set of popular full-stack frameworks:

- **Krab** (Rust)
- **Next.js** (React / Node.js)
- **Nuxt** (Vue / Node.js)
- **SvelteKit** (Svelte / Node.js)
- **Remix** (React / Node.js)
- **Leptos** (Rust / WASM)
- **Dioxus** (Rust / WASM)

### Route Matrix

Each framework implements the following standardized routes to ensure an apples-to-apples comparison:

| Route Key | Path | Type | Description |
|---|---|---|---|
| `home` | `/` | SSR Static | Server-rendered HTML with minimal hydration. Simulates a landing page. |
| `blog_post` | `/blog/:slug` | SSR Dynamic | Server-rendered HTML based on a URL parameter. Simulates content pages. |
| `api_status` | `/api/status` | JSON Read | A simple JSON API endpoint returning a static status object. |
| `api_mutation` | `/api/contact` | JSON Write | A POST endpoint accepting JSON payload, validating it, and returning a JSON response. |
| `health` | `/health` | Health Check | A lightweight liveness probe endpoint. |

## Execution Profiles

We run three distinct load profiles to measure different performance characteristics:

1.  **Load**: Steady-state traffic to measure baseline latency and throughput under normal conditions.
    *   Samples: 1,000
    *   Concurrency: 10
2.  **Spike**: Short bursts of high concurrency to measure resilience and recovery.
    *   Samples: 3,000
    *   Concurrency: 50
3.  **Soak**: Sustained traffic over a longer period to detect memory leaks or resource exhaustion.
    *   Samples: 10,000
    *   Concurrency: 20

## Fairness & Parity Rules

To ensure fairness, all benchmarks strictly adhere to the following rules:

1.  **Hardware**: All frameworks are tested on the same host class and OS image.
2.  **Resources**: CPU and memory limits are identical for all containers.
3.  **Network**: TLS termination and reverse proxy paths are identical.
4.  **Configuration**: Framework-specific debug mode is disabled (production builds only).
5.  **Payloads**: Response shapes and sizes are kept equivalent across implementations.

## Statistical Analysis

We report the following metrics for each run:

*   **p50 (Median)**: The typical latency experienced by 50% of requests.
*   **p95**: The latency experienced by 95% of requests (tail latency).
*   **p99**: The latency experienced by 99% of requests (extreme outliers).
*   **Mean**: The average latency.
*   **Error Rate**: The percentage of failed requests (non-2xx status codes).

## Reproducibility

Benchmarks are automated using the `scripts/nft_benchmark_runner.py` tool.

### Running Benchmarks Locally

1.  Start `service_frontend` with rate limiter thresholds appropriate for the chosen load profile. The default of 60 req/s will be saturated at benchmark concurrency, causing artificially high error rates.

    | Profile | `KRAB_RATE_LIMIT_REFILL_PER_SEC` | `KRAB_RATE_LIMIT_CAPACITY` |
    |---|---:|---:|
    | `load` | 2000 | 4000 |
    | `spike` | 10000 | 20000 |
    | `soak` | 5000 | 10000 |

    Example for the `load` profile:
    ```bash
    KRAB_RATE_LIMIT_REFILL_PER_SEC=2000 KRAB_RATE_LIMIT_CAPACITY=4000 cargo run -p service_frontend
    ```

2.  Start any other target frameworks on their configured ports (see [`plans/load_test_artifacts/benchmark_config.json`](../../plans/load_test_artifacts/benchmark_config.json)).

3.  Execute the runner:
    ```bash
    python3 scripts/nft_benchmark_runner.py --frameworks krab,nextjs --profile load
    ```
    If `KRAB_RATE_LIMIT_REFILL_PER_SEC`/`KRAB_RATE_LIMIT_CAPACITY` are not set when `krab` is a target, the runner will print a warning with the recommended values.

4.  Results are generated in [`plans/load_test_artifacts/external_results.json`](../../plans/load_test_artifacts/external_results.json) and `external_summary.md`.

## Artifacts

Benchmark data is stored in the [`plans/load_test_artifacts/`](../../plans/load_test_artifacts/) directory:

*   [`external_results.json`](../../plans/load_test_artifacts/external_results.json): Raw metrics for every run.
*   [`trend_history.csv`](../../plans/load_test_artifacts/trend_history.csv): Historical data for trend analysis.
*   [`external_summary.md`](../../plans/load_test_artifacts/external_summary.md): A human-readable summary of the latest run.
