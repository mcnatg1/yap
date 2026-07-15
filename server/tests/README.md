# Server tests

The server tier has portable contract/API/job-service/router/pool tests plus
the private checked-head GB10 inference boundary. The Phase 5 foreground
launcher contract is also checked without starting Docker or a service.

```text
tests/
  contract/
  api/
  jobs/
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

Portable CI validates the locks, command safety, durable job lifecycle and
recovery, bounded API behavior, queue behavior, worker input contract, output
attestation, WER logic, and atomic publication without loading Torch or
downloading a model. The clean-head private-node gate is documented in
`server/README.md` and `docs/runbooks/yap-server-node-setup.md`; it is the only
test boundary that builds the pinned ARM64 image and executes the Cohere model
on CUDA.
