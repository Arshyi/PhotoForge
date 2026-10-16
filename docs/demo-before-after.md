# Before and after pictures from the engine

`src-tauri/examples/demo_before_after.rs` makes a "before" and an "after" picture from one input
file and a short recipe, using the same engine code the application runs. It was written for a talk
(16 October 2026) and is useful whenever a picture of what an operation does is needed without
opening the window.

```powershell
cd src-tauri
cargo run --release --example demo_before_after -- recipe.json out-directory
```

It writes `<name>_before.png`, `<name>_after.png` and `<name>.json` (steps, sizes, the time each
step took, and any measurements) into the output directory.

## What the pictures are, and are not

* They are the output of `high_precision::apply` (the float pipeline every typed operation goes
  through) and of the layer compositor (for scaling). They are **not** screenshots of the
  application, and the user-interface path (panels, dialogs, undo) is not exercised.
* Scaling is the layer transform of the Transform panel, rendered by the compositor with
  **bilinear or nearest-neighbour** interpolation, the two a layer has. No editing operation
  resizes a picture any other way (previews and reduced opens do their own internal downscaling),
  no learned upscaler is included, and this tool does not add one; an enlarged picture contains no
  more information than the original.
* The false-colour pictures are ordinary Curves operations: a grey picture has equal red, green and
  blue, so one curve per channel is a colour table.

## Recipe format

```json
{
  "name": "bpae_recolour",
  "input": "C:/pictures/cells.jpg",
  "prepare": [ ],
  "before_steps": [ ],
  "steps": [
    { "kind": "select",
      "samples": [[560, 640], [575, 652]], "tolerance": 0.16,
      "luminance": 0.3, "hue": 1.0, "saturation": 0.3, "feather": 2,
      "operation": { "type": "hsl", "settings": {
        "master": { "hue": -80.0, "saturation": 0.0, "lightness": 0.0 },
        "red":     { "hue": 0.0, "saturation": 0.0, "lightness": 0.0 },
        "yellow":  { "hue": 0.0, "saturation": 0.0, "lightness": 0.0 },
        "green":   { "hue": 0.0, "saturation": 0.0, "lightness": 0.0 },
        "cyan":    { "hue": 0.0, "saturation": 0.0, "lightness": 0.0 },
        "blue":    { "hue": 0.0, "saturation": 0.0, "lightness": 0.0 },
        "magenta": { "hue": 0.0, "saturation": 0.0, "lightness": 0.0 } } } }
  ],
  "measure": [ { "label": "region", "region": [60, 330, 300, 100] } ]
}
```

* `prepare` steps are applied to both pictures (cropping a screenshot's window frame away, say),
  `before_steps` to the "before" picture alone (a nearest-neighbour scale, so a before and an after
  of different sizes can be compared at one size), `steps` to the "after".
* `{"kind": "op", "operation": { ... }}` is any typed `EditOperation`, with the same JSON a workflow
  or a plan carries (`"type": "denoise"`, `"levels"`, `"curves"`, `"local_contrast"`, ...). `hsl`
  needs every band written out.
* `{"kind": "scale", "factor": 5.0, "interpolation": "bilinear" | "nearest"}`.
* `{"kind": "select", ...}` makes a colour-range selection from the sample points (each of
  `tolerance`, `luminance`, `hue`, `saturation` between 0 and 1), optionally feathers it, and applies
  `operation` only inside it, as a masked operation. The report states how much of the picture the
  selection covered.
* `measure` regions report the mean and standard deviation of Rec. 709 luminance of the encoded
  8-bit picture before and after; they are only meaningful when the steps keep the size, and they are
  a description of one region, not a validation.

## The 2026-10-16 talk

Ten pairs were made this way (retinal OCT denoise, false colour and a five-times enlargement; a
fundus photograph; fluorescence recolouring and false colour; an H&E section; and three lab
pictures). Operations and parameters are printed on each slide. The public pictures came from
Wikimedia Commons and are credited on the final slide:

| Picture | Author | Licence |
| --- | --- | --- |
| SD-OCT Macula Cross-Section | Wies6014 | CC BY-SA 4.0 |
| Retina-OCT800 | medOCT group, Medical University of Vienna (2004) | CC BY 2.0 |
| Fundus photograph-normal retina EDA06 | National Eye Institute, NIH | public domain |
| Bovine Pulmonary Artery Endothelial Cells Fluorescent Image | Erin Rod | CC BY 4.0 |
| DAPI-stained nuclei of HaCaT keratinocytes | Accuratelibrarian | CC BY-SA 4.0 |
| Epidermis, papillary dermis and reticular dermis | Kilbad; Mikael Häggström, M.D. | CC0 |

The pictures themselves are not stored in the repository.
