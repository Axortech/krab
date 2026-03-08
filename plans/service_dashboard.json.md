# Service Dashboard Template (Grafana JSON Model)

This dashboard tracks RED (Rate, Errors, Duration) and USE (Utilization, Saturation, Errors) metrics for Krab services.

## Panels

### 1. Availability (SLO)
- **Metric:** `krab_response_5xx_total`, `krab_requests_total`
- **Query:** `sum(rate(krab_response_5xx_total[5m])) / (sum(rate(krab_requests_total[5m])) + 0.001)`
- **Visualization:** Time series (inverse, 100% - error rate)
- **Threshold:** < 99.9% (Alert)

### 2. Request Rate (Throughput)
- **Metric:** `krab_requests_total`
- **Query:** `rate(krab_requests_total[1m])`
- **Visualization:** Time series

### 3. Latency (Duration)
- **Metric:** `krab_request_duration_ms_bucket`, `krab_request_duration_ms_count`
- **Query:** `histogram_quantile(0.95, sum(rate(krab_request_duration_ms_bucket[5m])) by (le))`
- **Visualization:** Time series (p95, p99)
- **Threshold:** > 500ms (Warning)

### 4. Auth Failures
- **Metric:** `krab_auth_failures_total`
- **Query:** `rate(krab_auth_failures_total[5m])`
- **Visualization:** Time series (bars)

### 5. Dependency Health
- **Metric:** `krab_dependency_up` (To be implemented: 1 = Ready, 0 = Not Ready)
- **Visualization:** Stat/Gauge

## Alert Rules

1. **Availability burn (fast):** `AvailabilityBurnRateFast` — `rate(krab_response_5xx_total[5m]) / (rate(krab_requests_total[5m]) + 0.001) > 0.014` for 5m.
2. **Availability burn (slow):** `AvailabilityBurnRateSlow` — `rate(krab_response_5xx_total[30m]) / (rate(krab_requests_total[30m]) + 0.001) > 0.006` for 15m.
3. **Latency burn (fast):** `LatencyBurnRateFast` — request-over-500ms ratio > 10% for 5m.
4. **Latency burn (slow):** `LatencyBurnRateSlow` — request-over-500ms ratio > 5% for 15m.
5. **Auth burn (fast/slow):** `AuthBudgetBurnFast` and `AuthBudgetBurnSlow`.
6. **Dependency down:** `ServiceNotReady` — `krab_dependency_up{critical="true"} == 0` for 30s.
