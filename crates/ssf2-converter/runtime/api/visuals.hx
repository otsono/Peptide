// What the source does to how a thing looks, on the engine's object for it: a colour adjustment
// is a filter (the source's -100..100 are fractions here, its hue is degrees as here), a fade is
// its alpha, a tint is a colour shader. Null or false takes them off.

var Visuals = {};
// the engine's object a clip's colours are drawn with: its own, else the object it is part of
// (a fighter's animation clip tints the fighter)
Visuals.engineOf = function(target) {
	var c = target;
	var n = 0;
	while (c != null && n < 64) { var e = Stage.engineOf(c); if (e != null) return e; c = c.parent; n++; }
	return null;
};
Visuals.filtered = [];

Visuals.filters = function(target, o) {
	var t = Visuals.engineOf(target);
	if (t == null) return;
	for (r in Visuals.filtered) if (r.t == t) { if (r.f != null) t.removeFilter(r.f); Visuals.filtered.remove(r); break; }
	if (o == null || o == false) { t.setAlpha(1); return; }
	if (o.alphaMultiplier != null) t.setAlpha(AS3.num(o.alphaMultiplier));
	if (o.hue != null || o.saturation != null || o.brightness != null || o.contrast != null) {
		var h = new HsbcColorFilter();
		if (o.hue != null) h.hue = AS3.num(o.hue);
		if (o.saturation != null) h.saturation = AS3.num(o.saturation) / 100;
		if (o.brightness != null) h.brightness = AS3.num(o.brightness) / 100;
		if (o.contrast != null) h.contrast = AS3.num(o.contrast) / 100;
		t.addFilter(h);
		Visuals.filtered.push({ t: t, f: h });
	}
	var channels = o.redMultiplier != null || o.greenMultiplier != null || o.blueMultiplier != null
		|| o.redOffset != null || o.greenOffset != null || o.blueOffset != null;
	if (channels) Visuals.tint(target, ColorTransform.create(o.redMultiplier, o.greenMultiplier, o.blueMultiplier, 1,
		o.redOffset, o.greenOffset, o.blueOffset, 0));
};

// a colour transform on the engine's object: one shader each, changed in place
Visuals.shaded = [];
Visuals.tint = function(target, ct) {
	var t = Visuals.engineOf(target);
	if (t == null) return;
	// a container of the stage's art (a plane's clip) takes no shader here
	if (t.addShader == null) { PeptideRuntime.gap("a colour transform on a container of the stage's art"); return; }
	var sh = null;
	for (r in Visuals.shaded) if (r.t == t) sh = r.s;
	if (sh == null) {
		sh = new RgbaColorShader(1, 1, 1, 1, 0, 0, 0, 0);
		t.addShader(sh);
		Visuals.shaded.push({ t: t, s: sh });
	}
	sh.redMultiplier = AS3.num(ct.redMultiplier);
	sh.greenMultiplier = AS3.num(ct.greenMultiplier);
	sh.blueMultiplier = AS3.num(ct.blueMultiplier);
	sh.redOffset = AS3.num(ct.redOffset);
	sh.greenOffset = AS3.num(ct.greenOffset);
	sh.blueOffset = AS3.num(ct.blueOffset);
};
