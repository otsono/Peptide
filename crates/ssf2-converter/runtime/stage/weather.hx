// A stage's weather, as its own code makes it: the clips it puts in the weather clip
// (`getWeatherMC`) and moves every frame are drawn over the view. The source draws that clip on
// its own screen, so a place in it is a place on the screen, whatever the camera does; here the
// screen is the camera's view, and a clip in it is drawn at the same share of the view.
//
// What is drawn for a clip is the effect the stage pipeline made for its particle
// (`DATA.stage.particles`, by the particle clip's library name), at most `particleCap` of one
// particle at once.

var Weather = {};
// the source's screen, the space its weather clip is drawn in
Weather.view = { w: 640.0, h: 360.0 };
Weather.layer = null;
Weather.clip = function() {
	if (Weather.layer == null) Weather.layer = MovieClip.create();
	return Weather.layer;
};
// what is drawn for each clip in it: { c, v, p }
Weather.drawn = [];
Weather.counts = new haxe.ds.StringMap();
Weather.frames = 0;
// each weather field's drawn particles, by the field's effect: the lists the conversion declares
// as the stage's own variables (`DATA.stage.weatherLists`), filled in place
Weather.lists = null;
Weather.list = function(id) {
	if (Weather.lists == null) {
		Weather.lists = new haxe.ds.StringMap();
		for (w in PeptideRuntime.data.stage.weatherLists) Weather.lists.set(w.id, w.l);
	}
	var l = Weather.lists.get(id);
	if (l == null) { l = []; Weather.lists.set(id, l); }
	return l;
};

// the effect a clip is drawn as: the particle its class is, or none
Weather.particleOf = function(c) {
	for (p in PeptideRuntime.data.stage.particles) if (AS3.is(c, p.n)) return p;
	return null;
};

Weather.frame = function() {
	Weather.frames++;
	if (Weather.layer == null) {
		if (Weather.frames == 2 && DATA.stage.boot.weather.length > 0)
			PeptideRuntime.gap("the stage's weather is made by a class its package does not carry");
		return;
	}
	var cam = match.getCamera();
	if (cam == null) return;
	var vw = cam.getViewportWidth();
	var vh = cam.getViewportHeight();
	var kx = vw / Weather.view.w;
	var ky = vh / Weather.view.h;
	var left = cam.getX() - vw / 2;
	var topY = cam.getY() - vh / 2;
	var s = PeptideRuntime.data.units.size;
	for (d in Weather.drawn) d.seen = false;
	Weather.walk(Weather.layer, 0.0, 0.0, 1.0, 1.0, true, 1.0, { left: left, topY: topY, kx: kx, ky: ky, s: s });
	var keep = [];
	for (d in Weather.drawn) {
		if (d.seen) { keep.push(d); continue; }
		Weather.list(d.p.id).remove(d);
		m_owned.remove(d.v);
		d.v.destroy();
		d.c.__drawn = null;
		Weather.counts.set(d.p.n, Weather.counts.get(d.p.n) - 1);
	}
	Weather.drawn = keep;
};

// a clip and the ones it holds, each at its place in the clips over it
// (the view, fixed for the walk, travels as one value: the engine's script takes at most ten
// arguments to a call). A field is hundreds of clips a frame, so the walk reads a clip's place
// as it stands (a display object's place is always a number) and calls the engine only for
// what changed.
Weather.walk = function(c, x0, y0, sx0, sy0, shown0, a0, view) {
	for (k in c.__kids) {
		var kid = k.clip;
		if (kid == null) continue;
		var x = x0 + kid.x * sx0;
		var y = y0 + kid.y * sy0;
		var sx = sx0 * kid.scaleX;
		var sy = sy0 * kid.scaleY;
		var shown = shown0 && kid.visible != false;
		var a = a0 * kid.alpha;
		// what a clip is drawn as is worked out once, on the clip (false: nothing)
		if (kid.__wp == null) { var wp = Weather.particleOf(kid); kid.__wp = wp != null ? wp : false; }
		if (kid.__wp != false && kid.__skip != true) {
			var d = kid.__drawn != null ? kid.__drawn : Weather.make(kid, kid.__wp);
			if (d != null) {
				d.seen = true;
				d.v.setX(view.left + x * view.kx);
				d.v.setY(view.topY + y * view.ky);
				if (d.fresh == true || sx != d.sx || sy != d.sy || shown != d.shown || a != d.a) {
					d.fresh = false;
					d.sx = sx; d.sy = sy; d.shown = shown; d.a = a;
					d.v.setScaleX(sx * view.kx / view.s);
					d.v.setScaleY(sy * view.ky / view.s);
					d.v.setVisible(shown && a > 0);
					d.v.setAlpha(a);
				}
			}
		}
		if (kid.__kids != null && kid.__kids.length > 0) Weather.walk(kid, x, y, sx, sy, shown, a, view);
	}
};

// the effect for a clip that has none yet (past the cap, the clip is left undrawn)
Weather.make = function(c, p) {
	var n = Weather.counts.get(p.n);
	if (n == null) n = 0;
	if (n >= PeptideRuntime.data.stage.particleCap) {
		c.__skip = true;
		PeptideRuntime.gap("weather: more of one particle than are drawn at once");
		return null;
	}
	var v = match.createVfx(new VfxStats({ spriteContent: self.getResource().getContent(p.id), animation: "active", loop: true, timeout: -1, relativeWith: false }));
	if (v == null) return null;
	self.getBackgroundEffectsContainer().addChild(v.getViewRootContainer());
	m_owned.push(v);
	var d = { c: c, v: v, p: p, seen: true, fresh: true, sx: 1.0, sy: 1.0, shown: true, a: 1.0 };
	c.__drawn = d;
	Weather.drawn.push(d);
	Weather.list(p.id).push(d);
	Weather.counts.set(p.n, n + 1);
	return d;
};
