# Third-party notices

code-rcl is licensed under MIT. Its optional/default-enabled embedding runtime
and explicitly downloaded model have their own licenses:

| Component | License | Source |
|---|---|---|
| FastEmbed Rust 7.5.0 | Apache-2.0 | https://github.com/Anush008/fastembed-rs |
| ONNX Runtime | MIT | https://github.com/microsoft/onnxruntime |
| ort Rust bindings | MIT OR Apache-2.0 | https://github.com/pykeio/ort |
| Hugging Face tokenizers | Apache-2.0 | https://github.com/huggingface/tokenizers |
| Oniguruma | BSD-2-Clause | https://github.com/kkos/oniguruma |
| EmbeddingGemma 1 weights/tokenizer | Gemma terms | https://ai.google.dev/gemma/terms |

Model source: https://huggingface.co/google/embeddinggemma-300m

Pinned ONNX conversion:
https://huggingface.co/onnx-community/embeddinggemma-300m-ONNX/tree/5090578d9565bb06545b4552f76e6bc2c93e4a66

Model weights are downloaded separately by explicit setup and are not included
in code-rcl archives. Binary archives include this notice and the code-rcl
license. Build/distribution must retain licenses/notices accompanying bundled
native runtime artifacts. See Cargo.lock for the complete dependency inventory.

Windows archives also include unmodified Microsoft Visual C++ release
redistributable DLLs from the Visual Studio `VC/Redist` directory, under the
[Visual Studio redistribution terms](https://learn.microsoft.com/en-us/visualstudio/releases/2022/redistribution).
The Windows ONNX Runtime binary additionally depends on DirectML 1.15.4;
its Microsoft Software License Terms are preserved in
`licenses/bundled/DirectML-LICENSE.txt`. code-rcl uses CPU inference.
These Microsoft binaries are not covered by code-rcl's MIT license.
