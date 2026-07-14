# Phase 4 ASR Runtime Notices

The Phase 4 reference worker uses these pinned third-party artifacts:

- NVIDIA PyTorch container 26.06 for Linux ARM64, identified by the digest in
  `server/model-pools.lock.json`. Use is governed by the
  [NVIDIA Software License Agreement](https://www.nvidia.com/en-us/agreements/enterprise-software/nvidia-software-license-agreement/)
  and the
  [Product-Specific Terms for NVIDIA AI Products](https://www.nvidia.com/en-us/agreements/enterprise-software/product-specific-terms-for-ai-products/).
- `CohereLabs/cohere-transcribe-03-2026`, developed by Cohere and Cohere Labs,
  used without modification under Apache-2.0. The lock records the exact
  upstream identity, the public distribution revision, byte comparisons for
  the small runtime artifacts, matching upstream/distribution weight object
  identity and size, and the raw SHA-256 of every deployed artifact.
- The ASR gate fixture is a LibriSpeech sample distributed under CC BY 4.0.
  Its source, digest, and golden transcript are in
  `server/model-pools.lock.json`.

## Python overlay

The following resolver-minimal Python overlay is installed over the pinned
NVIDIA base. Versions and ARM64 wheel SHA-256 hashes are authoritative in
`server/runtime/asr/requirements.lock`; licenses and upstreams were verified
from the installed wheel metadata, bundled license files, and upstream project
license records.

| Distribution | Version | License | Upstream |
|---|---:|---|---|
| `audioread` | 3.1.0 | MIT | https://github.com/beetbox/audioread |
| `joblib` | 1.5.3 | BSD-3-Clause | https://github.com/joblib/joblib |
| `lazy-loader` | 0.5 | BSD-3-Clause | https://github.com/scientific-python/lazy-loader |
| `librosa` | 0.11.0 | ISC | https://github.com/librosa/librosa |
| `msgpack` | 1.2.1 | Apache-2.0 | https://github.com/msgpack/msgpack-python |
| `narwhals` | 2.24.0 | MIT | https://github.com/narwhals-dev/narwhals |
| `pooch` | 1.9.0 | BSD-3-Clause | https://github.com/fatiando/pooch |
| `scikit-learn` | 1.9.0 | BSD-3-Clause | https://github.com/scikit-learn/scikit-learn |
| `sentencepiece` | 0.2.1 | Apache-2.0 | https://github.com/google/sentencepiece |
| `soundfile` | 0.14.0 | BSD-3-Clause; bundled libsndfile is LGPL-2.1 | https://github.com/bastibe/python-soundfile |
| `soxr` | 1.1.0 | LGPL-2.1-or-later; bundled libsoxr/PFFFT notices included | https://github.com/dofuuz/python-soxr |
| `threadpoolctl` | 3.6.0 | BSD-3-Clause | https://github.com/joblib/threadpoolctl |
| `tokenizers` | 0.22.2 | Apache-2.0 | https://github.com/huggingface/tokenizers |
| `transformers` | 5.13.1 | Apache-2.0 | https://github.com/huggingface/transformers |

The `soundfile` wheel contains `libsndfile_arm64.so` and its LGPL-2.1 text.
The `soxr` wheel contains the libsoxr LGPL notice and the PFFFT license notice.
The pinned NVIDIA base supplies Torch/CUDA, Hugging Face Hub, NumPy, Protobuf,
Safetensors, and the remaining dependency closure under its own included
notices and governing terms.

The model remains an evaluation/reference pool. Yap does not imply endorsement
by NVIDIA, Hugging Face, OpenSLR, or the LibriSpeech authors.
