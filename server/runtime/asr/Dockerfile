FROM nvcr.io/nvidia/pytorch@sha256:dcae8df08ef61b019b8eb109113428cba4ef0e37484c6e722406150dd5ada759

LABEL org.opencontainers.image.title="Yap Phase 4 private batch ASR worker" \
      org.opencontainers.image.source="https://github.com/mcnatg1/yap" \
      org.opencontainers.image.licenses="LicenseRef-NVIDIA-AI AND Apache-2.0 AND BSD-3-Clause AND ISC AND LGPL-2.1-or-later AND MIT"

ENV PYTHONPATH=/opt/yap-server \
    YAP_MODEL_LOCK=/opt/yap-server/model-pools.lock.json \
    HF_HUB_OFFLINE=1 \
    TRANSFORMERS_OFFLINE=1 \
    HF_HUB_DISABLE_TELEMETRY=1 \
    DO_NOT_TRACK=1 \
    HOME=/tmp/yap-worker \
    HF_HOME=/tmp/huggingface \
    XDG_CACHE_HOME=/tmp/cache \
    TORCH_HOME=/tmp/torch \
    TMPDIR=/tmp \
    PYTHONDONTWRITEBYTECODE=1 \
    PYTHONUNBUFFERED=1

WORKDIR /opt/yap-server

COPY runtime/asr/requirements.lock /tmp/yap-asr-requirements.lock
RUN set -eu; \
    python3 -m pip install \
      --disable-pip-version-check \
      --no-cache-dir \
      --no-deps \
      --require-hashes \
      --requirement /tmp/yap-asr-requirements.lock; \
    check_status=0; \
    check_output="$(python3 -m pip check 2>&1)" || check_status=$?; \
    test "$check_status" -eq 1; \
    test "$check_output" = "triton-kernels 1.0.0+gitb7fa781f.nv26.6 requires pytest, which is not installed."; \
    rm -f /tmp/yap-asr-requirements.lock

COPY model-pools.lock.json /opt/yap-server/model-pools.lock.json
COPY runtime/asr/THIRD_PARTY_NOTICES.md /opt/yap-server/THIRD_PARTY_NOTICES.md
COPY src/yap_server /opt/yap-server/yap_server

RUN python3 -c 'import importlib.metadata as m, json, sys; import librosa, soundfile; from transformers import AutoProcessor, CohereAsrForConditionalGeneration; import torch; lock = json.load(open("/opt/yap-server/model-pools.lock.json", encoding="utf-8")); expected = lock["runtime"]["overlayPackages"]; actual = {name: m.version(name) for name in expected}; assert actual == expected, (actual, expected); assert sys.version_info[:2] == (3, 12); assert torch.__version__ == "2.13.0a0+8145d630e8.nv26.06"; assert torch.version.cuda == "13.3"' \
    && groupadd --gid 10001 yapworker \
    && useradd --uid 10001 --gid 10001 --no-create-home --shell /usr/sbin/nologin yapworker

USER 10001:10001
ENTRYPOINT ["python3", "-m", "yap_server.pools.batch_asr_worker"]
