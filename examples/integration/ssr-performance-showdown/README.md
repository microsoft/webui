# WebUI SSR Performance Test

Rust server that renders a spiral pattern of ~2400 tiles entirely on the
server, comparable to the `fastify-html` entry in the
[ssr-performance-showdown](https://github.com/nicolo-ribaudo/ssr-performance-showdown) benchmark.

The template is compiled once with `webui build` into a `protocol.bin`.
On every request the server computes the tile positions and passes them
as state to `WebUIHandler`, which streams the final HTML.

## Usage

```bash
# From the repository root — build the protocol first
cargo run -p webui-cli --release -- build examples/integration/ssr-performance-showdown/app --out examples/integration/ssr-performance-showdown/dist

# Run the server frrm this location
cargo run -p ssr-performance-showdown --release                 # listen on :3000
cargo run -p ssr-performance-showdown --release -- --port 3001  # custom port
```


## Results
Using `autocannon` we can run with warmup `npx autocannon -c 100 -d 10 -w 2 http://localhost:3000`:

**webui-rust**
```
┌─────────┬───────┬───────┬───────┬───────┬──────────┬─────────┬───────┐
│ Stat    │ 2.5%  │ 50%   │ 97.5% │ 99%   │ Avg      │ Stdev   │ Max   │
├─────────┼───────┼───────┼───────┼───────┼──────────┼─────────┼───────┤
│ Latency │ 11 ms │ 18 ms │ 44 ms │ 52 ms │ 21.71 ms │ 9.57 ms │ 87 ms │
└─────────┴───────┴───────┴───────┴───────┴──────────┴─────────┴───────┘
┌───────────┬────────┬────────┬────────┬────────┬─────────┬────────┬────────┐
│ Stat      │ 1%     │ 2.5%   │ 50%    │ 97.5%  │ Avg     │ Stdev  │ Min    │
├───────────┼────────┼────────┼────────┼────────┼─────────┼────────┼────────┤
│ Req/Sec   │ 2,987  │ 2,987  │ 4,623  │ 4,875  │ 4,511.3 │ 520.24 │ 2,986  │
├───────────┼────────┼────────┼────────┼────────┼─────────┼────────┼────────┤
│ Bytes/Sec │ 453 MB │ 453 MB │ 702 MB │ 740 MB │ 684 MB  │ 79 MB  │ 453 MB │
└───────────┴────────┴────────┴────────┴────────┴─────────┴────────┴────────┘
```

**fastify-html**
```
┌─────────┬───────┬───────┬────────┬────────┬──────────┬────────┬────────┐
│ Stat    │ 2.5%  │ 50%   │ 97.5%  │ 99%    │ Avg      │ Stdev  │ Max    │
├─────────┼───────┼───────┼────────┼────────┼──────────┼────────┼────────┤
│ Latency │ 87 ms │ 92 ms │ 107 ms │ 118 ms │ 93.42 ms │ 6.5 ms │ 135 ms │
└─────────┴───────┴───────┴────────┴────────┴──────────┴────────┴────────┘
┌───────────┬────────┬────────┬────────┬────────┬─────────┬───────┬────────┐
│ Stat      │ 1%     │ 2.5%   │ 50%    │ 97.5%  │ Avg     │ Stdev │ Min    │
├───────────┼────────┼────────┼────────┼────────┼─────────┼───────┼────────┤
│ Req/Sec   │ 915    │ 915    │ 1,075  │ 1,105  │ 1,060.7 │ 55.63 │ 915    │
├───────────┼────────┼────────┼────────┼────────┼─────────┼───────┼────────┤
│ Bytes/Sec │ 181 MB │ 181 MB │ 212 MB │ 218 MB │ 209 MB  │ 11 MB │ 181 MB │
└───────────┴────────┴────────┴────────┴────────┴─────────┴───────┴────────┘
```

**react**
```
┌─────────┬────────┬────────┬────────┬────────┬───────────┬──────────┬────────┐
│ Stat    │ 2.5%   │ 50%    │ 97.5%  │ 99%    │ Avg       │ Stdev    │ Max    │
├─────────┼────────┼────────┼────────┼────────┼───────────┼──────────┼────────┤
│ Latency │ 168 ms │ 180 ms │ 203 ms │ 210 ms │ 179.24 ms │ 16.77 ms │ 262 ms │
└─────────┴────────┴────────┴────────┴────────┴───────────┴──────────┴────────┘
┌───────────┬─────────┬─────────┬─────────┬─────────┬─────────┬────────┬─────────┐
│ Stat      │ 1%      │ 2.5%    │ 50%     │ 97.5%   │ Avg     │ Stdev  │ Min     │
├───────────┼─────────┼─────────┼─────────┼─────────┼─────────┼────────┼─────────┤
│ Req/Sec   │ 479     │ 479     │ 554     │ 573     │ 552     │ 25.95  │ 479     │
├───────────┼─────────┼─────────┼─────────┼─────────┼─────────┼────────┼─────────┤
│ Bytes/Sec │ 68.2 MB │ 68.2 MB │ 78.8 MB │ 81.5 MB │ 78.5 MB │ 3.7 MB │ 68.1 MB │
└───────────┴─────────┴─────────┴─────────┴─────────┴─────────┴────────┴─────────┘
```