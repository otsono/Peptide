// A stage's boot: what it puts down when the match starts, read off the converter's tables
// (`DATA.stage.boot`) and made in the order they list it. Its structures, its declared objects
// (the placed bodies first, before the fighters exist), its backdrop elements and the named clips
// they make up, its camera backgrounds' opacity, and its weather.
//
// Everything made goes on `__made` (what the running stage says it built), and the lists the
// stage's frame keeps (`m_owned`, `__follow`, `m_shown`, `m_tint`, `m_alpha`, `__platFollow`) and
// its name registry (`__nameObject`, `__objectNamed`, `__clipGroup`, `__panContainer`) are the
// stage script's own.

var StageBoot = {};
// what was made, by the tables' index: a declared object by its place in the stage's list, a
// backdrop element by its place in the element list
StageBoot.hz = [];
StageBoot.bg = [];

StageBoot.sized = function() {
	var b = DATA.stage.boot;
	while (StageBoot.hz.length < b.objectsDeclared) StageBoot.hz.push(null);
	while (StageBoot.bg.length < b.elements.length) StageBoot.bg.push(null);
};

// the bodies the tree placed: part of the stage from its first frame, ahead of the fighters
StageBoot.placed = function(owner) {
	var b = DATA.stage.boot;
	StageBoot.sized();
	var i = 0;
	while (i < b.objects.length) {
		var o = b.objects[i];
		if (o.placed == true) StageBoot.object(o, owner);
		i = i + 1;
	}
};

// everything else, once the match is live: structures, the objects (what the source puts behind
// the hazards switch only while hazards are on), then the backdrop elements
StageBoot.spawn = function(owner, chars, hazardsOn) {
	var b = DATA.stage.boot;
	StageBoot.sized();
	var i = 0;
	while (i < b.structures.length) {
		var s = b.structures[i];
		var h = match.createStructure(self.getResource().getContent(s.id));
		__made.push({ id: s.id, k: "s", o: h });
		if (s.name != null) __nameObject(s.name, h);
		i = i + 1;
	}
	var gated = false;
	i = 0;
	while (i < b.objects.length) {
		var o = b.objects[i];
		if (o.placed == true) {
			if (o.deck != null) StageBoot.deck(o, chars);
		} else if (o.gated == true) {
			gated = true;
		} else {
			StageBoot.object(o, owner);
		}
		i = i + 1;
	}
	if (gated) {
		if (hazardsOn) {
			i = 0;
			while (i < b.objects.length) {
				var o = b.objects[i];
				if (o.placed != true && o.gated == true) StageBoot.object(o, owner);
				i = i + 1;
			}
		}
	}
	i = 0;
	while (i < b.elements.length) {
		StageBoot.element(b.elements[i], owner);
		i = i + 1;
	}
};

// one declared object: made, put where it stands, drawn ahead of the fighters when it asks to be
// (with the separate piece it names riding it), and filed under the name the tree placed it by
StageBoot.object = function(o, owner) {
	var h = match.createCustomGameObject(self.getResource().getContent(o.id), owner);
	if (h != null) {
		h.setX(o.x);
		h.setY(o.y);
		__made.push({ id: o.id, k: "o", o: h });
	}
	if (o.front == true) {
		if (h != null) self.getForegroundStructuresContainer().addChild(h.getViewRootContainer());
		if (o.fg != null) {
			var v = match.createVfx(new VfxStats({ spriteContent: self.getResource().getContent(o.fg), animation: "active", x: 0, y: 0, loop: true, timeout: -1, relativeWith: false, resizeWith: false }));
			if (v != null) {
				self.getForegroundStructuresContainer().addChild(v.getViewRootContainer());
				__follow.push({ v: v, o: h });
			}
		}
	}
	if (o.name != null) __nameObject(o.name, h);
	StageBoot.hz[o.i] = h;
};

// the engine puts a fighter down on the stage's own floor, so one whose entrance is over a placed
// body starts inside it: it goes on the body's deck, where the source starts it
StageBoot.deck = function(o, chars) {
	var h = StageBoot.hz[o.i];
	if (h == null) return;
	var k = 0;
	while (k < chars.length) {
		var c = chars[k];
		if (c != null && c.getX() >= h.getX() + o.deck.l && c.getX() <= h.getX() + o.deck.r && c.getY() > h.getY() + o.deck.t) c.setY(h.getY() + o.deck.t);
		k = k + 1;
	}
};

// where an element's view goes
StageBoot.container = function(at) {
	if (at.pan == true) return __panContainer(at.px, at.py);
	if (at.plane == "camera") return match.getCamera().getBackgroundContainers()[at.n];
	if (at.plane == "background") return self.getBackgroundContainer();
	if (at.plane == "foreground") return self.getForegroundStructuresContainer();
	return self.getBackgroundBehindContainer();
};

// one backdrop element. A looping one is an effect in a container of its own, which the stage's
// timeline shows and hides; one whose clip switches sections is an object running its own script
StageBoot.element = function(e, owner) {
	var b = DATA.stage.boot;
	var v = null;
	// a piece that carries a named surface is that surface's structures, its art on the first
	if (e.carry != null) {
		var parts = [];
		for (cid in e.carry.ids) {
			var st = match.createStructure(self.getResource().getContent(cid));
			if (st == null) continue;
			st.setX(e.x);
			st.setY(e.y);
			parts.push(st);
			__made.push({ id: cid, k: "s", o: st });
		}
		Planes.carried.set(e.carry.n, parts);
		StageBoot.bg[e.i] = StageBoot.carrier(parts, e.sections);
		return;
	}
	// a picture placed inside an object's own clip that never changes is drawn in the object's own
	// view, where the engine carries it with the object (nothing in a named clip, which the code
	// reaches as a group)
	if (e.ride != null && e.ride.still == true && !StageBoot.grouped(e.i)) {
		var ro = StageBoot.hz[e.ride.o];
		var s = ro == null || ro.getViewRootContainer == null ? null : Sprite.create(self.getResource().getContent(e.id));
		if (s != null) {
			s.x = e.ride.dx;
			s.y = e.ride.dy;
			ro.getViewRootContainer().addChildAt(s, 0);
			return;
		}
	}
	if (e.seg == true) {
		v = match.createCustomGameObject(self.getResource().getContent(e.id), owner);
		if (v != null) {
			v.setX(e.x);
			v.setY(e.y);
			__made.push({ id: e.id, k: "o", o: v });
			var p = StageBoot.container(e.at);
			if (p != null) p.addChild(v.getViewRootContainer());
		}
	} else {
		v = match.createVfx(new VfxStats({ spriteContent: self.getResource().getContent(e.id), animation: "active", x: e.x, y: e.y, loop: true, timeout: -1, relativeWith: false, resizeWith: false }));
		var c = Container.create();
		StageBoot.container(e.at).addChild(c);
		if (v != null) {
			c.addChild(v.getViewRootContainer());
			m_owned.push(v);
			__made.push({ id: e.id, k: "v", o: v });
		}
		if (e.shown != null) m_shown.push({ c: c, i: 0, at: 0, on: e.shown });
		if (e.tint != null && v != null) {
			var sh = new RgbaColorShader(1, 1, 1, 1, 0, 0, 0, 0);
			v.addShader(sh);
			m_tint.push({ s: sh, i: 0, j: -1, at: 0, len: e.tlen, k: b.tints[e.tint] });
		}
		if (e.alpha != null && v != null) v.getViewRootContainer().alpha = e.alpha;
		if (e.hidden == true && v != null) v.setVisible(false);
	}
	StageBoot.bg[e.i] = v;
	// a piece placed inside an object's own clip is that object's picture: it rides the object
	if (e.ride != null && v != null) {
		var ao = StageBoot.hz[e.ride.o];
		if (ao != null && ao.getX != null) __follow.push({ v: v, o: ao, dx: e.ride.dx, dy: e.ride.dy });
	}
	// a surface its plane moves by label rides this picture
	if (e.follow != null && v != null) __platFollow.push({ v: v, n: e.follow.n, at: e.follow.at, bx: [], by: [] });
};

// the structures a piece carries, as the one piece the stage moves, shows and plays: each holds
// every section in one animation, a label at each section's start, so a section is played by its
// label and the one showing is the one the frame is in
StageBoot.carrier = function(parts, sec) {
	var h = { exports: {}, at: 0 };
	var each = function(f) { for (p in parts) if (!p.isDisposed()) f(p); };
	var start = function(k) { var s = 1; for (i in 0...k) s += sec.lens[i]; return s; };
	h.getX = function() { return parts.length > 0 ? parts[0].getX() : 0.0; };
	h.getY = function() { return parts.length > 0 ? parts[0].getY() : 0.0; };
	h.setX = function(x) { each(function(p) { p.setX(x); }); };
	h.setY = function(y) { each(function(p) { p.setY(y); }); };
	h.setVisible = function(v) { if (parts.length > 0) parts[0].setVisible(v); };
	h.setAlpha = function(a) { if (parts.length > 0) parts[0].setAlpha(a); };
	h.playAnimation = function(a) {
		var k = sec.anims.indexOf(a);
		if (k < 0) return;
		h.at = k;
		each(function(p) { p.playFrameLabel(a); p.resume(); });
	};
	h.playFrame = function(f) { var s = start(h.at); each(function(p) { p.playFrame(s + f - 1); }); };
	h.pause = function() { each(function(p) { p.pause(); }); };
	h.resume = function() { each(function(p) { p.resume(); }); };
	h.getAnimation = function() {
		if (parts.length == 0) return null;
		var f = parts[0].getCurrentFrame();
		var s = 1;
		for (i in 0...sec.lens.length) { if (f < s + sec.lens[i]) return sec.anims[i]; s += sec.lens[i]; }
		return sec.anims[sec.anims.length - 1];
	};
	h.getCurrentFrame = function() { return parts.length > 0 ? parts[0].getCurrentFrame() : 0; };
	h.getTotalFrames = function() { return parts.length > 0 ? parts[0].getTotalFrames() : 0; };
	h.isDisposed = function() { return parts.length == 0 || parts[0].isDisposed(); };
	return h;
};

// whether an element is a part of a named clip
StageBoot.grouped = function(i) {
	var gs = DATA.stage.boot.groups;
	var k = 0;
	while (k < gs.length) {
		if (gs[k].parts.indexOf(i) >= 0) return true;
		k = k + 1;
	}
	return false;
};

// a named clip: the object the tree placed under that name when there is one, else a group of
// the elements that came out of the clip
StageBoot.group = function(k) {
	var g = DATA.stage.boot.groups[k];
	if (__objectNamed(g.n) != null && __objectNamed(g.n).exports != null) return __objectNamed(g.n);
	var parts = [];
	var i = 0;
	while (i < g.parts.length) {
		parts.push(StageBoot.bg[g.parts[i]]);
		i = i + 1;
	}
	return __clipGroup(parts, g.labels, g.src, g.ws, g.plane);
};

// the camera backgrounds' opacity: a schedule the frame runs for one that fades, a constant for
// one that does not
StageBoot.camBg = function() {
	var b = DATA.stage.boot;
	var cbg = match.getCamera().getBackgroundContainers();
	var i = 0;
	while (i < b.camBg.length) {
		var e = b.camBg[i];
		if (cbg != null && cbg.length > e.i) {
			if (e.k != null) {
				m_alpha.push({ c: cbg[e.i], i: 0, j: -1, at: 0, len: e.len, k: e.k });
			} else {
				cbg[e.i].alpha = e.a;
			}
		}
		i = i + 1;
	}
};

// `lo + U(0, span) / 100`, the draw the tables state a spread as
StageBoot.draw = function(d) {
	if (d.span > 0) return d.lo + Random.getInt(0, d.span) / 100.0;
	if (d.span < 0) return d.lo - Random.getInt(0, -d.span) / 100.0;
	return d.lo;
};

// one weather field's particles, scattered over the view: as many as keep the source's density
// over this engine's screen, within the cap. Returns the field's particle list.
StageBoot.weather = function(k, alpha) {
	var w = DATA.stage.boot.weather[k];
	var cam = match.getCamera();
	var shw = w.hw;
	var shh = w.hh;
	if (cam != null) {
		shw = cam.getViewportWidth() * 0.75;
		shh = cam.getViewportHeight() * 0.75;
	}
	var wn = w.count;
	if (w.area != null) wn = Std.int(w.count * (shw * shh) / w.area);
	if (wn < w.count) wn = w.count;
	if (wn > w.cap) wn = w.cap;
	var list = [];
	var n = 0;
	while (n < wn) {
		var v = match.createVfx(new VfxStats({ spriteContent: self.getResource().getContent(w.id), animation: "active", loop: true, timeout: -1, relativeWith: false }));
		// every particle solid: the source fades the whole field, never one drop
		var pa = 1.0;
		v.setAlpha(pa * alpha);
		if (w.scale != null) {
			var ps = StageBoot.draw(w.scale);
			v.setScaleX(ps);
			v.setScaleY(ps);
		}
		self.getBackgroundEffectsContainer().addChild(v.getViewRootContainer());
		m_owned.push(v);
		var px = (Random.getInt(0, 200) / 100.0 - 1.0) * shw;
		var py = (Random.getInt(0, 200) / 100.0 - 1.0) * shh;
		var pk = StageBoot.draw(w.dy);
		var pw = StageBoot.draw(w.dx);
		list.push({ v: v, a: pa, x: px, y: py, k: pk, wind: pw });
		n = n + 1;
	}
	return list;
};
