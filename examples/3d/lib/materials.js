// Named materials for presentation renders, loaded with
//
//   const mat = await cad.use("materials");
//   cad.render({ quality: "presentation", materials: mat.option(), ... });
//
// The core knows no material names: cad.render's `materials` option maps
// the values of one attribute to physical numbers (colour, metallic,
// roughness, transmission, index of refraction, clear coat, a pattern).
// This file is where the names live. Parts say what they are made of with
// an attribute (`material: "oak"` by default), and option() turns the
// presets below into the map.
//
// Colours are sRGB. Patterns run along a part's longest direction unless
// they name an axis, so a shelf's grain runs along the shelf and a side's
// up the side, however the part was modelled.

// Wood: a light, slightly oiled surface with growth rings.
function wood(color, ringColor, { spacing = 3.5, strength = 0.55, roughness = 0.55, coat = 0.2 } = {}) {
  return {
    color,
    roughness,
    clearcoat: coat,
    clearcoatRoughness: 0.35,
    pattern: { kind: "bands", axis: "long", spacing, color: ringColor, strength, warp: 0.7 * spacing, distance: 35 * spacing },
  };
}

// Metal brushed along its length.
function brushed(color, roughness, { anisotropy = 0.75, streaks = 0.5, scale = 0.08 } = {}) {
  return {
    color,
    metallic: 1,
    roughness,
    pattern: { kind: "brushed", axis: "long", anisotropy, streaks, scale },
  };
}

// Metal turned on a lathe or faced: streaks in circles about the part's
// axis of symmetry.
function turned(color, roughness, { anisotropy = 0.6, streaks = 0.5, scale = 0.05 } = {}) {
  return {
    color,
    metallic: 1,
    roughness,
    pattern: { kind: "brushed", axis: "symmetry", circular: true, anisotropy, streaks, scale },
  };
}

// Paint: matte by default, glossy under a clear coat.
function painted(color, { gloss = false } = {}) {
  return gloss
    ? { color, roughness: 0.45, clearcoat: 1, clearcoatRoughness: 0.05 }
    : { color, roughness: 0.85 };
}

// Moulded plastic: a smooth dielectric.
function plastic(color, { roughness = 0.38 } = {}) {
  return { color, roughness, ior: 1.46 };
}

// Glazed ceramic: a coloured body under a smooth, thick glaze.
function glazed(color, { roughness = 0.45, gloss = 0.03 } = {}) {
  return { color, roughness, clearcoat: 1, clearcoatRoughness: gloss };
}

const presets = {
  oak: wood("#b8894f", "#7a5530"),
  walnut: wood("#6e4a2e", "#3e2716", { spacing: 3 }),
  // Birch plywood: pale, fine rings.
  ply: wood("#dcc39a", "#b39466", { spacing: 1.6, strength: 0.4, coat: 0.1 }),
  // Pressure-treated, stained timber for outdoor frames.
  timber: wood("#9a7550", "#5f4329", { spacing: 6, strength: 0.5, roughness: 0.75, coat: 0 }),
  steel: brushed("#c5c8cb", 0.3),
  stainless: brushed("#d0d2d4", 0.22),
  aluminium: brushed("#dcdee0", 0.26, { anisotropy: 0.6 }),
  "turned steel": turned("#c5c8cb", 0.28),
  "turned aluminium": turned("#dcdee0", 0.24),
  brass: brushed("#d6b06a", 0.25),
  glass: { color: "#ffffff", transmission: 1, roughness: 0, ior: 1.5 },
  // Printed circuit board under green solder mask.
  pcb: { color: "#1f5a32", roughness: 0.4, clearcoat: 0.6, clearcoatRoughness: 0.15 },
  "black plastic": plastic("#262729"),
  "grey plastic": plastic("#8e9296"),
  "white plastic": plastic("#e6e6e2"),
  "white paint": painted("#ecebe6"),
  "red paint": painted("#b3261e", { gloss: true }),
  "green paint": painted("#2f6b3a", { gloss: true }),
  rubber: { color: "#202020", roughness: 0.9 },
  "white glaze": glazed("#ece8de"),
  "blue glaze": glazed("#2f5d8a"),
  // A lawn, for a ground: rough green with soft, metre-sized patches of
  // drier grass.
  lawn: {
    color: "#557f36", roughness: 0.95,
    pattern: { kind: "mottle", scale: 1800, strength: 0.6, color: "#8c9241" },
  },
  // Polished steel: a mirror finish with fine circular polishing marks.
  "polished steel": turned("#d4d6d8", 0.08, { anisotropy: 0.3, streaks: 0.25 }),
};

// The `materials` option for cad.render: every preset, plus `extra`
// (more names, or replacements), looked up by the attribute `attr`.
function option(extra = {}, attr = "material") {
  return { attr, map: { ...presets, ...extra } };
}

return { presets, option, wood, brushed, turned, painted, plastic, glazed };
