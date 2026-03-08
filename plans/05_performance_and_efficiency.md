# Krab Framework: Performance & Efficiency Plan

## 1. Philosophy: "Fast by Default, Efficient by Design"
Krab aims to be the most performant full-stack framework in the Rust ecosystem. This isn't just about raw request-per-second (RPS) metrics, but also about developer feedback loops (compile times), memory footprint, and end-user interaction latency (TTI).

## 2. Build-Time Optimizations (Developer Experience)
Slow compile times are the #1 complaint in Rust web development. Krab addresses this aggressively.

### A. Parallel Dual-Target Compilation
The Krab CLI (`krab dev` / `krab build`) orchestrates the build process:
- **Concurrency**: Spawns separate threads/processes to compile the Server (native) and Client (WASM) targets simultaneously.
- **Smart Rebuilds**:
  - If only server code changes, the WASM build is skipped.
  - If only a specific "island" changes, we aim to only rebuild that island (future goal: separate crates per island for true incremental builds).

### B. Compiler Tuning
- **Development Profile**:
  - **Backend**: Usage of the `cranelift` codegen backend for debug builds (significantly faster compilation speed at the cost of runtime performance).
  - **Linker**: Defaults to `mold` (Linux) or `zld` (macOS) / `lld` (Windows) for instant linking.
  - **Incremental Compilation**: Aggressively enabled.
- **Release Profile**:
  - **LTO**: "Fat" LTO for maximum optimization.
  - **Codegen Units**: Set to 1 for maximum optimization (at the cost of build time).
  - **Panic**: `abort` to reduce binary size.

### C. WASM Post-Processing
- **wasm-opt**: Integrated into the release pipeline to run `wasm-opt -Oz` (aggressively optimize for size).
- **Dead Code Elimination**: Since islands are specific entry points, aggressive tree-shaking is applied to remove unused framework code from the client bundle.

## 3. Server-Side Runtime Performance
Leveraging Rust's zero-cost abstractions to minimize overhead.

### A. Zero-Copy Architecture
- **Serialization**: Prefer `rkyv` over `serde_json` for internal data exchange. `rkyv` guarantees zero-copy deserialization, meaning data can be read directly from the raw bytes without allocation.
- **String Handling**: Extensive use of `Cow<'a, str>` and `Bytes` to avoid unnecessary cloning of request/response bodies.

### B. Memory Management
- **Arena Allocation**: For request-scoped data (e.g., template rendering contexts), use bump allocation (via `bumpalo`). This is significantly faster than standard heap allocation and improves cache locality.
- **Object Pooling**: Reusing expensive objects (buffers, database connections) to reduce allocator pressure.

### C. The "Krab" Runtime
- **Tokio Tuning**: Custom configuration of the Tokio runtime, optimizing for high concurrency I/O.
- **Hyper Integration**: leveraging `hyper`'s low-level APIs for maximum throughput, bypassing higher-level overhead where unnecessary.

## 4. Client-Side Efficiency (The Browser)
Minimizing the "Tax" sent to the user's device.

### A. The "Islands" Advantage
- **Zero JS by Default**: Static pages ship 0kb of JavaScript.
- **Granular Bundling**: Each island is a separate entry point (or grouped efficiently).
- **Result**: Drastically smaller initial download size compared to frameworks like Yew or Dioxus (full SPA mode).

### B. Fine-Grained Reactivity
- **Signals**: Direct DOM updates.
  - Changing a signal value updates *only* the specific text node or attribute bound to it.
  - **No Virtual DOM**: Eliminates the overhead of diffing a virtual tree against the real DOM.
  - **Memory Efficiency**: Lower memory usage than VDOM-based frameworks which maintain a shadow tree.

### C. Lazy Hydration strategies
- **Idle**: Hydrate low-priority islands when the main thread is idle (`requestIdleCallback`).
- **Visible**: Hydrate only when the island enters the viewport (Intersection Observer).
- **Interaction**: Hydrate only when the user hovers or clicks (e.g., a "Buy Now" button).

## 5. Network & Data Protocol
Optimizing the wire format.

### A. Data Serialization
- **Server -> Client**: Data needed for hydration is serialized efficiently.
  - **Format**: `rkyv` (binary) encoded in Base64 or raw bytes, embedded in the HTML.
  - **Benefit**: Parsing binary data is orders of magnitude faster than parsing large JSON blobs in the browser.

### B. Asset Delivery
- **Compression**: Brotli (level 11 for static, level 4 for dynamic) and Gzip support out of the box.
- **HTTP/3**: Experimental support for QUIC via `h3` / `quinn` integration in the future.

## 6. Benchmarking & Limits
We will maintain a suite of benchmarks to ensure no regressions.
- **Binary Size Limit**: Warn if a simple "Hello World" island exceeds 10kb (compressed).
- **RPS Target**: Aim to be within 90% of raw `hyper` performance for simple echoes.
- **Startup Time**: Sub-millisecond cold start (crucial for Serverless/Edge).
