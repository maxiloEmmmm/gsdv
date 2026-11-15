---
name: gsdv-image
description: "Show a local image in the active GSDV frontend"
---

Use this only when `GSDV_HOOK_ENDPOINT` exists and the user should inspect a local image.

1. Resolve the image to an absolute filesystem path.
2. Run `~/.gsdv/hooks/show-image /absolute/path/to/image`.
3. Report command errors normally. Do not embed or copy the image into the conversation merely to trigger GSDV.
