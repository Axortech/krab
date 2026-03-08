# Public Assets

This directory contains static assets served by `krab_server`.

## Building Client

To generate the client-side WASM and JS files, run:

```bash
cd ../../krab_client
wasm-pack build --target web --out-dir ../service_frontend/public --no-typescript
```

This will generate `krab_client.js`, `krab_client_bg.wasm`, and `package.json` in this directory.
