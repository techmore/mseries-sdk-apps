# Apple Silicon Linux diffusion: upstream evidence and patch plan

Reviewed October 22, 2026. Target: 32 GB M1 Pro, elementary OS on Asahi/Aurora;
Linux Vulkan is the relevant GPU backend. macOS Metal/MPS measurements do not
qualify this host. Keep the protected 6 GiB desktop reserve.

## Current evidence

- Read-only host inventory: running kernel `7.2.11-elementary-aurora72-wip3`,
  `mesa-vulkan-drivers 26.0.8-1ubuntu0.3`, `libvulkan1 1.4.341.0-1`.
  Package versions identify the installed stack; they do not qualify a newer
  upstream compute patch. The historical `.20` recovery baseline is distinct
  from this running kernel.
- Local coordinator reports: Realistic Vision with CPU VAE completed in about
  258 seconds. The following GPU-VAE trial stopped after about 3 seconds at the
  reserve gate. These are distinct configurations; GPU VAE is not qualified.
- [stable-diffusion.cpp](https://github.com/leejet/stable-diffusion.cpp/tree/fd21ecfceef2665d2c5f2db8536e3202d864fd30)
  is the app's pinned native engine. Its [build instructions](https://github.com/leejet/stable-diffusion.cpp/blob/fd21ecfceef2665d2c5f2db8536e3202d864fd30/docs/build.md)
  support `-DSD_VULKAN=ON`. Preserve its patched GGML: substituting upstream GGML
  can remove operators and performance optimizations and changes FP8 loading.
- [Realistic Vision V6 B1 producer recipe](https://huggingface.co/SG161222/Realistic_Vision_V6.0_B1_noVAE/blob/9a857a696b9aabbf509073e0aa55ec8200b6ef7d/README.md)
  recommends the separate `stabilityai/sd-vae-ft-mse-original` VAE and DPM++ SDE
  Karras at 25+ steps, or DPM++ 2M SDE at 50+. Larger portrait dimensions are
  recommended, with lower resolution when anatomy degrades. Eight-step Euler
  does not represent that published quality recipe. Resolution increases and
  Hires.Fix still require measured memory admission on this device.

## Related upstreams and limits

**Aurora** means [omacom/linux-aurora](https://github.com/omacom/linux-aurora),
with the repository's historical `.20` baseline. Its DMA heap/udmabuf candidate
[6984b8c07e84b0cf528243db1838dade971ff513](https://github.com/omacom/linux-aurora/commit/6984b8c07e84b0cf528243db1838dade971ff513)
requires 16K alignment, permissions and explicit application support. Existing
`mseries/aurora/enhancements/README.md` makes no local-AI performance claim.
Kernel changes are not an app optimization prerequisite.

**Asahi/Honeykrisp** supplies the Vulkan driver. API conformance establishes
compatibility, not equivalent diffusion throughput to Metal. The
[Asahi Vulkan announcement](https://asahilinux.org/2024/06/vk13-on-the-m1-in-1-month/)
is an API milestone, not a Stable Diffusion benchmark. Its later
[Linux 6.16 progress report](https://asahilinux.org/2025/08/progress-report-6-16/)
records that the Mesa fork was retired after upstreaming. Track maintained
Mesa changes rather than treating the retired fork as the current driver.

**omarchy-mlx** at
[790e7dfe016e757992c65b398484aa424396691f](https://github.com/joshuaswarren/omarchy-mlx/tree/790e7dfe016e757992c65b398484aa424396691f)
implements MLX using Vulkan. Its README lists M1 Pro as untested. The
[engine ledger](https://github.com/joshuaswarren/omarchy-mlx/blob/790e7dfe016e757992c65b398484aa424396691f/docs/ENGINES.md)
reports llama.cpp token-throughput gains; these are not still-image timings.
It also qualifies TensorFold MiniMax-H3 video, but its
[video guide](https://github.com/joshuaswarren/omarchy-mlx/blob/790e7dfe016e757992c65b398484aa424396691f/docs/tensorfold-video.md)
requires 96 GB and records failure on 64 GB. It is unsuitable for this 32 GB
machine. Its query-chunked SDPA patch avoids a 2 GiB storage-buffer binding
limit; that is a TensorFold/MLX implementation, not a drop-in sd.cpp patch.

The proposed [Mesa compute stack PR #6](https://github.com/omacom/mesa/pull/6),
reviewed at head `f144c7949b7d960f26677b671341d5a123057ddf`, explicitly says
**NOT READY TO MERGE**. An earlier wide CTS recorded 55 regressions; newer
48-case targeted passes do not replace the pending wide suite. Do not install
this stack based on its LLM speedups or claim it accelerates diffusion.

## Safe application/runtime candidates

1. Preserve the completed CPU-VAE configuration as the measured baseline;
   retain GPU VAE as failed admission, not a faster supported preset.
2. Record engine SHA, Vulkan device/driver, model and VAE receipts, every actual
   sampler setting, first-preview latency, total duration and measured peak
   memory in each run. Compare only matching prompt, seed and dimensions.
   Repeated recipes now discard previous run measurements before execution.
   Process RSS excludes GPU allocations; do not label it total memory usage.
3. Add the producer RV recipe as a quality candidate, then qualify dimensions
   incrementally within the reserve. Model weight size alone does not establish
   fit; activations, VAE and vision encoder count too.
4. Reference consistency candidate: SD1.5 IP-Adapter Plus Face with explicit
   `--backend te=cpu,clip_vision=cpu`. `--clip-on-cpu` affects only text encoding.
   [h94/IP-Adapter revision 018e402774aeeddd60609b4ecdb7e298259dc729](https://huggingface.co/h94/IP-Adapter/tree/018e402774aeeddd60609b4ecdb7e298259dc729/models)
   has a 2,528,373,448-byte vision encoder and a 98,183,288-byte Plus Face
   adapter, Apache 2.0. No native fp16 encoder sibling exists there. Generic
   sd.cpp fp16 conversion is source-supported but converted-vision loading and
   its actual memory peak must be verified before declaring this usable.
5. SDXL base is a subsequent quality candidate, not yet qualified. FLUX schnell
   fp16's approximately 24 GB transformer alone leaves insufficient room under
   this reserve before encoders/activations; quantized variants need independent
   compatibility and peak-memory checks.

No kernel, bootloader, audio-stack or global Mesa changes were made for this
review. No upstream diffusion performance number establishes speed on this
particular M1 Pro; use retained local measurements.

## Port decisions from source review (no new runs)

The backend uses the pinned engine's [module backend contract](https://github.com/leejet/stable-diffusion.cpp/blob/fd21ecfceef2665d2c5f2db8536e3202d864fd30/docs/backend.md):
`diffusion=vulkan0,te=cpu,clip_vision=cpu,vae=cpu` (or `vae=vulkan0`), with
explicit parameter storage and a managed GPU budget. This is the Linux Apple GPU
path; macOS MPS/Metal install instructions are not used.

| Component | Use in this port | Settings/dependencies |
|---|---|---|
| Realistic Vision V6 FP16 | Main photographic checkpoint | External MSE VAE; DPM++ 2M/Karras 30-step app preset is separate from the author's DPM++ SDE 25+ / 2M SDE 50+ recipe. |
| SD 1.5 Q4/Q8 | Lower-storage alternative | Same SD 1.5 reference family; Euler 20/CFG7 starter, not the RV quality recipe. |
| SDXL Turbo | Composition/speed option | Author 512-square, 1–4 steps, no CFG/negative conditioning; sd.cpp CFG1. SD 1.5 reference adapter rejected. |
| IP-Adapter Plus Face SD 1.5 | Appearance consistency | Matching ViT-H encoder; strength0.6–0.8 upstream starting range. It is appearance conditioning, not an identity guarantee. |
| TAESD SD 1.x | Live preview only | `--taesd-preview-only --preview tae`; saved final uses the selected full VAE. |
| Qwen3-VL 2B + matching projector | Optional feedback | Separate CPU review after generation; never auto-started by the port. |

The [SDXL Turbo author limitations](https://huggingface.co/stabilityai/sdxl-turbo/blob/main/README.md)
explicitly include imperfect faces/photorealism; it is not the primary portrait recommendation.
SDXL base and quantized FLUX remain researched candidates, not silently enabled choices.

Kernel disposition: retain the installed Aurora/Asahi stack. DMA heap/udmabuf in
[Aurora PR67](https://github.com/omacom/linux-aurora/pull/67) requires applications
that use those allocation paths; this sd.cpp port does not, so enabling the config
is not a diffusion acceleration patch. The M2 Max/G14X compute-preemption candidate
is not an M1 Pro optimization (see the repository's existing
`aurora/UPSTREAM-BRANCH-AUDIT-2026-10-17.md`). Mesa PR6 remains experimental per its
maintainer. No kernel/boot/Mesa mutation is required for decoder/reference support.

Review limit: source compatibility, model cards, pinned CLI definitions and patch
status establish what can be ported. They do not establish this host's speed or
quality for an unrun model. No generations, benchmark suites or model conversions
were executed during this source-integration pass.
