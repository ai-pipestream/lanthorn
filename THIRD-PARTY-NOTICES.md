# Third-Party Notices

This document lists:

1. **Derived code:** Open-source projects from which lanthorn has derived code or data structures.
2. **Bundled components:** Third-party software bundled and distributed with the Docker image.

## Derived Code

### glulxe

**Project:** glulxe  
**Copyright:** © 1999–2023, Andrew Plotkin  
**License:** MIT  
**URL:** https://github.com/erkyrath/glulxe  
**Commit read:** Read from master branch, 2026-09-09 (Bocfel migration audit SQ-1444)  
**Lanthorn files:**
- `crates/gvm/src/exec.rs` — floating-point opcodes (`op_fmod`, `op_dmodr`, `op_dmodq`, `op_ftonumz`, `op_dtonumz`, `op_dtonumn`, `glulx_powf` wrapper)

### MIT License

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

## Bundled Components

### Recall model

Recall uses Sentence Transformers **all-MiniLM-L6-v2**, revision
`1110a243fdf4706b3f48f1d95db1a4f5529b4d41` (Apache-2.0).
The executable does not contain weights: a native install downloads them on
first use. The Docker image bundles the model's pinned `config.json`,
`tokenizer.json`, and `model.safetensors`, verified against
`crates/app/src/recall/model-manifest.json` during the build. The image also
contains `LICENSE` (the Apache 2.0 text, verified against the manifest) and
`MODEL.txt` (model, revision, model card, and artifact hashes) in
`/usr/local/share/lanthorn/recall-model/`.

- Model card and license declaration: https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2
- License text: https://www.apache.org/licenses/LICENSE-2.0.txt
- Inference uses Hugging Face Candle (MIT OR Apache-2.0) and Tokenizers
  (Apache-2.0). The BERT masked-mean pooling implementation follows Candle's
  BERT example and the model's Sentence Transformers pooling configuration.
- Candle: https://github.com/huggingface/candle
- Tokenizers: https://github.com/huggingface/tokenizers

### Docker components

These components are bundled and distributed as part of the Docker image (`docker build -t lanthorn .`).

### ttyd

**Project:** ttyd  
**Distribution:** Upstream release binary (see `Dockerfile` lines 63–84)  
**URL:** https://github.com/tsl0922/ttyd  
**Release:** 1.7.7 from https://github.com/tsl0922/ttyd/releases/download/1.7.7/ttyd.*  
**License:** MIT  
**Copyright:** © 2016–2026, Shuanglei Tao  

**Notes:** The Docker image serves the TUI through ttyd when run in server mode (`docker run lanthorn serve`). No lanthorn code is derived from ttyd.

#### MIT License

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

### dtach

**Project:** dtach  
**Distribution:** Debian trixie package 0.9-7 (see `Dockerfile` line 130)  
**URL:** https://github.com/crigler/dtach  
**License:** GPL-2.0  

**Notes:** The Docker image uses dtach to manage sessions that survive websocket drops (SQ-1323). dtach is bundled unmodified from the Debian trixie distribution package and is invoked as an external process. No lanthorn code is derived from dtach.
