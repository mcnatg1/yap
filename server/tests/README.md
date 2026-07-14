# Server tests

The server tier has portable contract/API/router/pool tests plus one private
GB10 Phase 4 inference gate.

```text
tests/
  contract/
  api/
  workload_router/
  model_pools/
  infra/
  fixtures/asr/
```

Run the portable suite with Python 3.12:

```powershell
$env:PYTHONPATH = "server/src"
python -m unittest discover -s server/tests -p "test_*.py"
```

Portable CI validates the locks, command safety, queue behavior, worker input
contract, output attestation, WER logic, and atomic publication without loading
Torch or downloading a model. The clean-head private-node gate is documented in
`server/README.md` and `docs/runbooks/yap-server-node-setup.md`; it is the only
test that builds the pinned ARM64 image and executes the Cohere model on CUDA.
