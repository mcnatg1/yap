# Phase 5 ASR Runtime Evaluation

**Snapshot:** 2026-07-14

**Decision:** Keep the checked NVIDIA PyTorch 26.06, Python 3.12, CUDA 13.3,
Transformers, and BF16 path as the executable Phase 5 baseline. Treat NVIDIA
vLLM 26.06 as the only current ASR performance challenger. Do not promote a
vLLM or quantized path until it passes the same model identity, transcript
quality, resource, process-cleanup, and Yap contract gates. SGLang remains a
candidate for later text/agent pools, not the Cohere ASR pool.

This record evaluates runtime choices. It does not authorize an image pull,
replace the locked Phase 4/5 runtime, or claim performance that Yap has not
measured on the GB10.

## Yap Requirements

The relevant requirements are narrower than choosing the fastest general LLM
server:

- Linux ARM64 on DGX Spark GB10 with Python 3.12;
- the exact Cohere ASR revision and its 14-language contract;
- offline, hash-verified model artifacts with no worker network access;
- mono PCM16/16 kHz batch input and Yap-owned create/upload/commit/status/result
  semantics;
- one bounded MVP worker today, with measured concurrency later;
- transcript accuracy and provenance ahead of speculative low-bit savings; and
- no runtime/model port exposed to the desktop, LAN, or public internet.

The model/runtime adapter is replaceable. A replacement still has to preserve
the Yap API contract and pass license, artifact, WER, language, cancellation,
restart, retention, and process-containment evidence. A model or runtime label
alone is never sufficient.

## Candidates

| Candidate | What it provides | Fit for current Cohere batch slice | Main cost or risk |
| --- | --- | --- | --- |
| NVIDIA PyTorch `26.06-py3` + Transformers/BF16 | Python 3.12, CUDA 13.3, prerelease NVIDIA Torch 2.13, Torch-TensorRT 2.13, TensorRT 11, Model Optimizer, and a flexible PyTorch execution surface | Best known-correct baseline. Yap has already pinned its ARM64 digest, overlay, model artifacts, WER fixture, isolation command, and GB10 result | Cold model/container startup per job and no continuous batching; the broad framework image needs Yap's overlay and containment |
| NVIDIA vLLM `26.06-py3` | Python 3.12, vLLM 0.22.1, Transformers 5.6, Torch 2.13, CUDA 13.3, continuous batching, and an OpenAI-compatible transcription API | Best next performance experiment. Upstream vLLM lists `CohereAsrForConditionalGeneration` and the exact Cohere Transcribe model as supported | No Yap GB10 accuracy/latency/cleanup evidence yet. The 26.06 release warns that default GPU allocation is too aggressive on unified-memory systems such as DGX Spark |
| NVIDIA SGLang `26.06-py3` | Python 3.12, SGLang 0.5.12.post1, Torch 2.13, CUDA 13.3, DGX Spark support, and FP8/NVFP4 features for supported models | Useful later for Yap's text generation or agent workloads | Current NVIDIA and SGLang model surfaces do not document the Cohere Transcribe ASR architecture or a matching transcription API, so it is not the Phase 5 ASR runtime |
| Triton with a vLLM Python backend | A larger multi-backend serving and observability surface | Plausible after Yap needs several persistent pools and operations tooling | Extra service, packaging, health, scheduling, and attack surface for a one-model/one-worker MVP; it does not remove Yap's ownership and contract layer |
| Torch-TensorRT or a custom TensorRT export | Potential engine-level latency and memory improvements | A later optimization experiment if the Cohere encoder-decoder graph exports cleanly | Model-specific conversion, dynamic generation, calibration, accuracy parity, and artifact provenance are unproven |

## Why BF16 Remains The Default

The locked Cohere model is a 2-billion-parameter BF16 checkpoint of about
4.13 GB. That is not a memory-pressure reason by itself to accept an unverified
quantized derivative on the GB10. The existing BF16 path has executable WER,
runtime-attestation, artifact-hash, and cleanup evidence; the low-bit paths do
not.

vLLM supports several quantization families and Blackwell-oriented formats in
general, while the Cohere model page lists community quantized derivatives.
Neither fact proves that a specific Cohere ASR quantization preserves Yap's
WER, multilingual behavior, punctuation, long-audio stability, or licensing
and byte provenance. Quantization is therefore benchmark-gated rather than a
Phase 5 default.

The first challenger should be the unquantized BF16 model under vLLM. That
isolates the serving-engine effect. Only if measured memory, throughput, or
latency creates a real need should the same harness evaluate FP8 or a
Blackwell-native four-bit format. Every candidate needs a newly pinned model
artifact identity; a community checkpoint is not inherited as trusted merely
because it names the canonical model as its base.

## Promotion Benchmark

A vLLM or quantized candidate can replace the baseline only after a disposable,
digest-pinned GB10 run records all of the following against the same audio set:

1. exact model/revision and container/runtime identities;
2. WER and punctuation parity by supported language, not only one English clip;
3. cold-start time, warm real-time factor, p50/p95 latency, peak unified memory,
   and sustained one/two/three-job throughput;
4. long-audio behavior through Yap's four-hour admission boundary and practical
   shorter fixtures;
5. cancellation, timeout, restart, queue saturation, and clean container/process
   teardown;
6. identical Yap result authority, hashes, replay behavior, and error semantics;
7. offline artifact loading and proof that no runtime/model port becomes a
   client-facing interface; and
8. license, notice, digest, vulnerability-review, and model-provenance records.

For NVIDIA vLLM 26.06 on DGX Spark, the benchmark must explicitly set and record
`--gpu-memory-utilization`; NVIDIA documents that the default is effectively
near 1.0 and may cause unified-memory OOM failures. The correct value is a
measurement result, not a hard-coded architectural constant.

## Sources

- [NVIDIA PyTorch 26.06 release notes](https://docs.nvidia.com/deeplearning/frameworks/pytorch-release-notes/rel-26-06.html)
- [NVIDIA vLLM 26.06 release notes](https://docs.nvidia.com/deeplearning/frameworks/vllm-release-notes/rel-26-06.html)
- [vLLM supported transcription models](https://docs.vllm.ai/en/v0.21.0/models/supported_models/#transcription)
- [vLLM online speech-to-text APIs](https://docs.vllm.ai/en/stable/serving/online_serving/#speech-to-text-apis)
- [NVIDIA SGLang 26.06 release notes](https://docs.nvidia.com/deeplearning/frameworks/sglang-release-notes/rel-26-06.html)
- [Cohere Transcribe model card](https://huggingface.co/CohereLabs/cohere-transcribe-03-2026)
- [vLLM quantization overview](https://docs.vllm.ai/en/stable/features/quantization/)
- [LLM Compressor scheme selection](https://docs.vllm.ai/projects/llm-compressor/en/latest/steps/choosing-scheme/)
