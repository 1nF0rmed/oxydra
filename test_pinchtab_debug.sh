#!/bin/bash
# Comprehensive Pinchtab debugging test script
# Tests the full path: Gateway WS → Runtime → shell_exec → shell-daemon → container → pinchtab

set -u

GATEWAY_ENDPOINT="ws://127.0.0.1:34787/ws"
BRIDGE_TOKEN="90754d27abbd040495f44d7a00b01cbcd9c33d98645de8bebb855420be0eb3dd"
USER_ID="shaan"

echo "=== TEST 1: Direct Pinchtab health from inside shell-vm ==="
time docker exec oxydra-container-shaan-shell-vm curl -sf --max-time 5 \
  -H "Authorization: Bearer $BRIDGE_TOKEN" \
  http://127.0.0.1:9867/health 2>&1
echo ""

echo ""
echo "=== TEST 2: Direct navigate from inside shell-vm ==="
time docker exec oxydra-container-shaan-shell-vm curl -sf --max-time 30 \
  -X POST http://127.0.0.1:9867/navigate \
  -H "Authorization: Bearer $BRIDGE_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"url":"https://example.com"}' 2>&1
echo ""

echo ""
echo "=== TEST 3: Direct snapshot from inside shell-vm ==="
time docker exec oxydra-container-shaan-shell-vm curl -sf --max-time 30 \
  -H "Authorization: Bearer $BRIDGE_TOKEN" \
  "http://127.0.0.1:9867/snapshot?filter=interactive&maxTokens=2000" 2>&1
echo ""

echo ""
echo "=== TEST 4: Direct text from inside shell-vm ==="
time docker exec oxydra-container-shaan-shell-vm curl -sf --max-time 30 \
  -H "Authorization: Bearer $BRIDGE_TOKEN" \
  "http://127.0.0.1:9867/text" 2>&1
echo ""

echo ""
echo "=== TEST 5: Navigate to a more complex site ==="
time docker exec oxydra-container-shaan-shell-vm curl -sf --max-time 60 \
  -X POST http://127.0.0.1:9867/navigate \
  -H "Authorization: Bearer $BRIDGE_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"url":"https://news.ycombinator.com"}' 2>&1
echo ""

echo ""
echo "=== TEST 6: Snapshot of complex site ==="
time docker exec oxydra-container-shaan-shell-vm curl -sf --max-time 30 \
  -H "Authorization: Bearer $BRIDGE_TOKEN" \
  "http://127.0.0.1:9867/snapshot?filter=interactive&maxTokens=2000&format=compact" 2>&1
echo ""

echo ""
echo "=== TEST 7: Shell daemon responsiveness - simple echo ==="
time docker exec oxydra-container-shaan-shell-vm echo "shell-vm is responsive"
echo ""

echo ""
echo "=== TEST 8: Execute curl through shell daemon from oxydra-vm perspective ==="
# This tests the shell-daemon socket path
time docker exec oxydra-container-shaan-oxydra-vm bash -c '
  echo "Testing shell daemon socket connection..."
  ls -la /workspace/ipc/shell-daemon.sock 2>/dev/null || echo "no socket at /workspace/ipc/"
  ls -la /tmp/shell-daemon.sock 2>/dev/null || echo "no socket at /tmp/"
  find / -name "shell-daemon.sock" 2>/dev/null || echo "no socket found"
' 2>&1
echo ""

echo ""
echo "=== TEST 9: Check container resource usage ==="
docker stats --no-stream --format "table {{.Name}}\t{{.CPUPerc}}\t{{.MemUsage}}\t{{.MemPerc}}\t{{.NetIO}}\t{{.BlockIO}}" oxydra-container-shaan-oxydra-vm oxydra-container-shaan-shell-vm
echo ""

echo ""
echo "=== TEST 10: Check container logs for errors ==="
echo "--- shell-vm logs (last 30 lines) ---"
docker logs --tail 30 oxydra-container-shaan-shell-vm 2>&1
echo ""
echo "--- oxydra-vm logs (last 30 lines) ---"
docker logs --tail 30 oxydra-container-shaan-oxydra-vm 2>&1
echo ""

echo ""
echo "=== TEST 11: Multiple rapid requests to Pinchtab ==="
for i in $(seq 1 5); do
  echo -n "Request $i: "
  time docker exec oxydra-container-shaan-shell-vm curl -sf --max-time 10 \
    -H "Authorization: Bearer $BRIDGE_TOKEN" \
    http://127.0.0.1:9867/health 2>&1
done

echo ""
echo "=== TEST 12: Navigate then immediately snapshot (timing) ==="
echo "--- Navigate ---"
time docker exec oxydra-container-shaan-shell-vm curl -sf --max-time 30 \
  -X POST http://127.0.0.1:9867/navigate \
  -H "Authorization: Bearer $BRIDGE_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"url":"https://httpbin.org/get"}' 2>&1
echo ""
echo "--- Immediate snapshot ---"
time docker exec oxydra-container-shaan-shell-vm curl -sf --max-time 30 \
  -H "Authorization: Bearer $BRIDGE_TOKEN" \
  "http://127.0.0.1:9867/text" 2>&1
echo ""

echo ""
echo "=== Done ==="
