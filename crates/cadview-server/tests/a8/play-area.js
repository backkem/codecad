// A play area for the free-space rule: equipment footprints are closed
// rectangles (mm) with attributes the script chooses: what the item is and
// how much free space it needs around it.

const site = cad.addPolyline(
  [[0, 0], [12000, 0], [12000, 8000], [0, 8000]],
  { closed: true, layer: "SITE", attrs: { role: "site" } },
);
const item = (name, x, y, w, h, free) =>
  cad.addPolyline(
    [[x, y], [x + w, y], [x + w, y + h], [x, y + h]],
    { closed: true, layer: "PLAY", attrs: { item: name, free } },
  );

item("tower", 1000, 1000, 1500, 1500, 1500);
item("sandbox", 2600, 1000, 1200, 1200, 0); // 100 from the tower
item("swing", 5000, 1000, 3000, 2000, 2000);
item("bench", 6000, 2500, 1500, 400, 0); // overlaps the swing
item("slide", 9500, 6000, 3000, 600, 2000); // runs past the site edge

return cad.describe().entities;
