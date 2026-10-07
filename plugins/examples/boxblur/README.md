Box Blur averages each pixel with the pixels within its radius.

It declares a local reach of four pixels, so the renderer gives it four pixels of context around every tile and the result does not show tile edges. The plugin manager verifies that claim by running the filter whole and in tiles and comparing.
