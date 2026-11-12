// The pieces a stage's boot puts down and the upkeep its frame runs, as Fraymakers code: a stage
// written natively carries only the blocks it uses (`emit/stage.rs`). The conversion writes the
// boot as one call per piece, in the order the stage puts things down.

// ---- sized
//: the handles the boot files by index (a piece riding another is found by it)
var bootBg = [];
// the structures each piece carrying a named surface made, by the surface's name
var carried = new haxe.ds.StringMap();
function bootSized(objects, elements) {
	while (bootBg.length < elements) bootBg.push(null);
}

// ---- structure
//: a structure the stage entity carries
function bootStructure(id, name) {
	var h = match.createStructure(self.getResource().getContent(id));
	__made.push({ id: id, k: "s", o: h });
	if (name != null) nameObject(name, h);
}

// ---- container
//: where an element's view goes
function bootContainer(at) {
	if (at.pan == true) return panContainer(at.px, at.py);
	if (at.plane == "camera") return match.getCamera().getBackgroundContainers()[at.n];
	if (at.plane == "background") return self.getBackgroundContainer();
	if (at.plane == "foreground") return self.getForegroundStructuresContainer();
	return self.getBackgroundBehindContainer();
}

// ---- pan
//: one container per pan rate, moved against the camera by that rate: a piece at 0 stays with the
//: view, one at 1 stays in the world
var panRates = [];
var panConts = [];
var panX0 = null;
var panY0 = null;
function panContainer(px, py) {
	for (i in 0...panRates.length) if (panRates[i].x == px && panRates[i].y == py) return panConts[i];
	var c = Container.create();
	self.getBackgroundBehindContainer().addChild(c);
	panRates.push({ x: px, y: py });
	panConts.push(c);
	return c;
}
// the pan containers, against where the camera started (PAN_X, PAN_Y: the size they centre on)
function panUpkeep() {
	if (panConts.length == 0) return;
	var cam = match.getCamera();
	if (cam == null) return;
	if (panX0 == null) { panX0 = cam.getX(); panY0 = cam.getY(); }
	var dx = cam.getX() - panX0;
	var dy = cam.getY() - panY0;
	for (i in 0...panConts.length) {
		panConts[i].x = cam.getX() - PAN_X * 0.5 - dx * panRates[i].x;
		panConts[i].y = cam.getY() - PAN_Y * 0.5 - dy * panRates[i].y;
	}
}

// ---- effect
//: a looping piece: an effect in a container of its own, which the stage's timeline shows and hides
function bootEffect(e) {
	var v = match.createVfx(new VfxStats({ spriteContent: self.getResource().getContent(e.id), animation: "active", x: e.x, y: e.y, loop: true, timeout: -1, relativeWith: false, resizeWith: false }));
	var c = Container.create();
	bootContainer(e.at).addChild(c);
	if (v != null) {
		c.addChild(v.getViewRootContainer());
		m_owned.push(v);
		__made.push({ id: e.id, k: "v", o: v });
	}
	if (e.shown != null) m_shown.push({ c: c, i: 0, at: 0, on: e.shown });
	if (e.tint != null && v != null) {
		var sh = new RgbaColorShader(1, 1, 1, 1, 0, 0, 0, 0);
		v.addShader(sh);
		m_tint.push({ s: sh, i: 0, j: -1, at: 0, len: e.tlen, k: e.tint });
	}
	if (e.alpha != null && v != null) v.getViewRootContainer().alpha = e.alpha;
	if (e.hidden == true && v != null) v.setVisible(false);
	bootRides(e, v);
}

// ---- cam_alpha
//: a camera background's opacity
function bootCamAlpha(i, a) {
	var cbg = match.getCamera().getBackgroundContainers();
	if (cbg != null && cbg.length > i) cbg[i].alpha = a;
}

// ---- cam_fade
//: a camera background's opacity as a schedule the frame runs
function bootCamFade(i, len, k) {
	var cbg = match.getCamera().getBackgroundContainers();
	if (cbg != null && cbg.length > i) m_alpha.push({ c: cbg[i], i: 0, j: -1, at: 0, len: len, k: k });
}

// ---- shown
//: shown over runs of [from, to) on the clock: a cursor per container, moved on only forward
function shownUpkeep(t) {
	for (e in m_shown) {
		if (t < e.at) e.i = 0;
		e.at = t;
		while (e.i < e.on.length && t >= e.on[e.i][1]) e.i++;
		e.c.visible = e.i < e.on.length && t >= e.on[e.i][0];
	}
}

// ---- key
//: opacity and colour hold each key until the next, on the piece's own loop
function keyAt(e, c) {
	if (c < e.at) e.i = 0;
	e.at = c;
	while (e.i + 1 < e.k.length && c >= e.k[e.i + 1][0]) e.i++;
}

// ---- alphas
function alphasUpkeep(t) {
	for (e in m_alpha) {
		var c = e.len > 0 ? t % e.len : t;
		keyAt(e, c);
		e.c.alpha = e.k[e.i][1];
	}
}

// ---- tints
function tintsUpkeep(t) {
	for (e in m_tint) {
		var c = e.len > 0 ? t % e.len : t;
		keyAt(e, c);
		var k = e.k[e.i];
		e.s.redMultiplier = k[1];
		e.s.greenMultiplier = k[2];
		e.s.blueMultiplier = k[3];
		e.s.redOffset = k[4];
		e.s.greenOffset = k[5];
		e.s.blueOffset = k[6];
	}
}

// ---- named
//: what the stage files under a name, as its code (and its objects' code, through
//: `exports.__named`) asks for one: a structure the boot made, else a region the tree placed
//: (REGIONS: each region's rectangle and the point its clip is placed by, this engine's units)
var namedObjs = [];
var stageRegions = REGIONS;
var regionsMade = [];
function nameObject(n, o) {
	if (o != null) namedObjs.push({ n: n, o: o });
}
function objectNamed(n) {
	for (e in namedObjs) if (e.n == n) return e.o;
	// (a named surface, where the stage has any: SURFACES_ON)
	if (SURFACES_ON) { var sf = surfaceNamed(n); if (sf != null) return sf; }
	for (r in regionsMade) if (r.name == n) return r;
	for (r in stageRegions) {
		if (r.n != n) continue;
		// (a region its code moves moves its rectangle with it)
		var rect = { x: r.x, y: r.y, w: r.w, h: r.h };
		var e = { name: n, x: r.rx, y: r.ry, rect: rect };
		e.getX = function() { return e.x; };
		e.getY = function() { return e.y; };
		e.setX = function(v) { rect.x += v - e.x; e.x = v; };
		e.setY = function(v) { rect.y += v - e.y; e.y = v; };
		e.isDisposed = function() { return false; };
		regionsMade.push(e);
		return e;
	}
	return null;
}

// ---- rides
//: what a piece is to the rest of the stage: its handle by index, the object it rides, and the
//: surface its plane moves by label riding it
var bootHz = [];
var platFollow = [];
function bootRides(e, v) {
	bootBg[e.i] = v;
	if (e.ride != null && v != null) {
		var ao = bootHz[e.ride.o];
		if (ao != null && ao.getX != null) __follow.push({ v: v, o: ao, dx: e.ride.dx, dy: e.ride.dy });
	}
	if (e.follow != null && v != null) platFollow.push({ v: v, n: e.follow.n, at: e.follow.at, bx: [], by: [] });
}

// ---- follow
//: what rides an object follows it
function followUpkeep() {
	for (f in __follow) {
		if (f.v == null || f.o == null) continue;
		f.v.setX(f.o.getX() + (f.dx != null ? f.dx : 0));
		f.v.setY(f.o.getY() + (f.dy != null ? f.dy : 0));
	}
}

// ---- segment
//: a piece whose clip switches sections: an object running its own script
function bootSegment(e, owner) {
	var v = match.createCustomGameObject(self.getResource().getContent(e.id), owner);
	if (v != null) {
		v.setX(e.x);
		v.setY(e.y);
		__made.push({ id: e.id, k: "o", o: v });
		var p = bootContainer(e.at);
		if (p != null) p.addChild(v.getViewRootContainer());
	}
	bootRides(e, v);
}

// ---- sectioned
//: a sectioned piece drawn as effects: the section playing is one effect, and at its end an engine
//: timer plays the section that follows (or the section's last frame, held, where the source
//: stops); the stage plays a section on it by name
function bootSectioned(e, c) {
	var sec = e.sections;
	var h = { exports: {}, v: null, k: -1, t: -1, dx: 0.0, dy: 0.0 };
	var drop = function(v) { if (v != null && !v.isDisposed()) v.kill(); };
	var show = function(anim, loop) {
		var v = match.createVfx(new VfxStats({ spriteContent: self.getResource().getContent(e.id), animation: anim, x: e.x, y: e.y, loop: loop, timeout: -1, relativeWith: false, resizeWith: false }));
		if (v != null) c.addChild(v.getViewRootContainer());
		return v;
	};
	h.play = function(k) {
		if (h.t >= 0) { self.removeTimer(h.t); h.t = -1; }
		drop(h.v);
		h.k = k;
		var n = e.chain[k];
		h.v = show(sec.anims[k], n == k);
		if (n == k) return;
		h.t = self.addTimer(sec.lens[k], 1, function() {
			h.t = -1;
			if (n >= 0) { h.play(n); return; }
			var was = h.v;
			h.v = show(sec.anims[k] + "_hold", true);
			drop(was);
		}, { persistent: true });
	};
	h.getX = function() { return e.x + h.dx; };
	h.getY = function() { return e.y + h.dy; };
	h.setX = function(x) { h.dx = x - e.x; c.x = h.dx; };
	h.setY = function(y) { h.dy = y - e.y; c.y = h.dy; };
	h.setVisible = function(v) { c.visible = v; };
	h.getVisible = function() { return c.visible; };
	h.setAlpha = function(a) { c.alpha = a; };
	h.playAnimation = function(a) { var k = sec.anims.indexOf(a); if (k >= 0) h.play(k); };
	h.playFrame = function(f) {};
	h.pause = function() { if (h.v != null && !h.v.isDisposed()) h.v.pause(); };
	h.resume = function() { if (h.v != null && !h.v.isDisposed()) h.v.resume(); };
	h.getAnimation = function() { return h.k < 0 ? null : sec.anims[h.k]; };
	h.getCurrentFrame = function() { return h.v == null || h.v.isDisposed() ? 0 : h.v.getCurrentFrame(); };
	h.getTotalFrames = function() { return h.v == null || h.v.isDisposed() ? 0 : h.v.getTotalFrames(); };
	h.isDisposed = function() { return false; };
	if (e.hidden == true) c.visible = false;
	h.play(0);
	return h;
}

// ---- chained
//: a sectioned piece in a camera background: effects in that background's container
function bootChained(e) {
	var c = Container.create();
	var at = bootContainer(e.at);
	if (at == null) return;
	at.addChild(c);
	var h = bootSectioned(e, c);
	__made.push({ id: e.id, k: "o", o: h });
	bootBg[e.i] = h;
}

// ---- carry
//: a piece that carries a named surface: that surface's structures, its art on the first, as the one
//: piece the stage moves, shows and plays (each holds every section in one animation, a label at
//: each section's start, so a section is played by its label)
function bootCarry(e) {
	var parts = [];
	for (cid in e.carry.ids) {
		var st = match.createStructure(self.getResource().getContent(cid));
		if (st == null) continue;
		st.setX(e.x);
		st.setY(e.y);
		parts.push(st);
		__made.push({ id: cid, k: "s", o: st });
	}
	carried.set(e.carry.n, parts);
	bootBg[e.i] = bootCarrier(parts, e.sections);
}
function bootCarrier(parts, sec) {
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
}

// ---- still
//: a picture placed inside an object's own clip that never changes is drawn in the object's own
//: view, where the engine carries it with the object; one whose object has no view is drawn as an
//: effect riding it
function bootStill(e, owner) {
	var ro = bootHz[e.ride.o];
	var s = ro == null || ro.getViewRootContainer == null ? null : Sprite.create(self.getResource().getContent(e.id));
	if (s == null) {
		if (e.seg == true) bootSegment(e, owner); else bootEffect(e);
		return;
	}
	s.x = e.ride.dx;
	s.y = e.ride.dy;
	ro.getViewRootContainer().addChildAt(s, 0);
}

// ---- object
//: one declared object: made, put where it stands, drawn ahead of the fighters when it asks to be
//: (with the separate piece it names riding it), and filed under the name the tree placed it by
function bootObject(o, owner) {
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
	if (o.name != null) nameObject(o.name, h);
	while (bootHz.length <= o.i) bootHz.push(null);
	bootHz[o.i] = h;
}

// ---- deck
//: the engine puts a fighter down on the stage's own floor, so one whose entrance is over a placed
//: body starts inside it: it goes on the body's deck, where the source starts it
function bootDeck(o, chars) {
	var h = bootHz[o.i];
	if (h == null) return;
	var k = 0;
	while (k < chars.length) {
		var c = chars[k];
		if (c != null && c.getX() >= h.getX() + o.deck.l && c.getX() <= h.getX() + o.deck.r && c.getY() > h.getY() + o.deck.t) c.setY(h.getY() + o.deck.t);
		k = k + 1;
	}
}

// ---- group
//: a named clip that came out as pieces: the object the tree placed under that name when there is
//: one, else the pieces, shown, hidden and faded together
var stageGroups = new haxe.ds.StringMap();
function bootGroup(g) {
	var named = null;
	for (e in namedObjs) if (e.n == g.n) named = e.o;
	if (named != null && named.exports != null) return named;
	var parts = [];
	for (i in g.parts) parts.push(bootBg[i]);
	var grp = { parts: parts, labels: g.labels, name: g.src, widths: g.ws, plane: g.plane, visible: true };
	grp.setVisible = function(v) {
		grp.visible = v;
		for (p in parts) if (p != null) p.setVisible(v);
	};
	grp.setAlpha = function(a) {
		for (p in parts) if (p != null) p.setAlpha(a);
	};
	return grp;
}

// ---- surfaces
//: a surface the stage entity carries under a name (SURFACES: each name and its structures' names):
//: its structures, moved and switched together, standing where the tree placed its clip; its code
//: can give it a ground friction as the match runs (an ice floor)
var surfaceRows = SURFACES;
var surfacesMade = new haxe.ds.StringMap();
var codedFriction = [];
function surfaceNamed(n) {
	var have = surfacesMade.get(n);
	if (have != null) return have;
	var row = null;
	for (r in surfaceRows) if (r.n == n) row = r;
	if (row == null) return null;
	var sts = [];
	var find = function() {
		if (sts.length == 0 && carried.exists(n)) for (st in carried.get(n)) sts.push(st);
		if (sts.length == 0) for (k in row.s) { var st = match.getStructureByName(k); if (st != null) sts.push(st); }
		return sts;
	};
	var at = null;
	for (r in stageRegions) if (r.n == n) at = r;
	var e = { name: n, x: at == null ? 0.0 : at.rx, y: at == null ? 0.0 : at.ry, exports: {} };
	e.getX = function() { return e.x; };
	e.getY = function() { return e.y; };
	e.setX = function(v) { var d = v - e.x; e.x = v; for (st in find()) st.setX(st.getX() + d); };
	e.setY = function(v) { var d = v - e.y; e.y = v; for (st in find()) st.setY(st.getY() + d); };
	e.isDisposed = function() { return false; };
	e.updateStructureStats = function(o) { for (st in find()) st.updateStructureStats(o); };
	e.getStructureStat = function(k) { var l = find(); return l.length > 0 ? l[0].getStructureStat(k) : null; };
	e.addToBlacklist = function(o) { for (st in find()) st.addToBlacklist(o); };
	e.removeFromBlacklist = function(o) { for (st in find()) st.removeFromBlacklist(o); };
	e.structures = find;
	e.setDecelFriction = function(v) { e.decel = v * ACCEL; if (codedFriction.indexOf(e) < 0) codedFriction.push(e); };
	e.setAccelFriction = function(v) { e.accel = v * ACCEL; if (codedFriction.indexOf(e) < 0) codedFriction.push(e); };
	surfacesMade.set(n, e);
	return e;
}

// ---- ground
//: the pieces with a ground friction of their own (a fighter standing in one slows by it, and gets
//: its own back on leaving) and the ones with a sideways push (a fighter standing in one is moved):
//: FRICTIONS and PUSHES, each a box in this engine's units
var frictionRows = FRICTIONS;
var pushRows = PUSHES;
var frictionHeld = [];
function groundUpkeep() {
	if (frictionRows.length == 0 && pushRows.length == 0 && codedFriction.length == 0) return;
	var ps = match.getCharacters();
	for (i in 0...ps.length) {
		var p = ps[i];
		while (frictionHeld.length <= i) frictionHeld.push(null);
		var on = null;
		if (p.isOnFloor()) {
			var x = p.getX();
			var y = p.getY();
			for (b in frictionRows) if (x >= b.x0 && x <= b.x1 && y >= b.y0 && y <= b.y1) on = b;
			if (codedFriction.length > 0) {
				var floor = p.getCurrentFloor();
				for (c in codedFriction) if (floor != null && c.structures().indexOf(floor) >= 0) {
					var f0 = p.getCharacterStat("friction");
					var a0 = p.getCharacterStat("walkSpeedAcceleration");
					var held = frictionHeld[i];
					on = { f: c.decel != null ? c.decel : (held != null ? held.f : f0), a: c.accel != null ? c.accel : (held != null ? held.a : a0) };
				}
			}
			for (b in pushRows) if (x >= b.x0 && x <= b.x1 && y >= b.y0 && y <= b.y1) p.setX(p.getX() + b.dx);
		}
		if (on != null) {
			if (frictionHeld[i] == null) frictionHeld[i] = { f: p.getCharacterStat("friction"), a: p.getCharacterStat("walkSpeedAcceleration") };
			if (p.getCharacterStat("friction") != on.f || p.getCharacterStat("walkSpeedAcceleration") != on.a)
				p.updateCharacterStats({ friction: on.f, walkSpeedAcceleration: on.a });
		} else if (frictionHeld[i] != null) {
			p.updateCharacterStats({ friction: frictionHeld[i].f, walkSpeedAcceleration: frictionHeld[i].a });
			frictionHeld[i] = null;
		}
	}
}

// ---- riding
//: a surface whose plane moves it by label rides its own picture: its structures sit where the
//: picture's current frame has moved from where it was put
function ridingUpkeep() {
	for (e in platFollow) {
		if (e.v == null) continue;
		var l = (e.v.exports != null && e.v.exports.__label != null) ? e.v.exports.__label : e.v.getAnimation();
		var d = null;
		for (a in e.at) if (a.l == l) d = a.d;
		if (d == null || d.length == 0) continue;
		var j = Std.int((e.v.getCurrentFrame() - 1) / 2);
		if (j < 0) j = 0;
		if (j * 2 + 1 >= d.length) j = Std.int(d.length / 2) - 1;
		for (r in surfaceRows) if (r.n == e.n) {
			for (s in 0...r.s.length) {
				var st = match.getStructureByName(r.s[s]);
				if (st == null) continue;
				if (e.bx.length <= s) { e.bx.push(st.getX()); e.by.push(st.getY()); }
				st.setX(e.bx[s] + d[j * 2]);
				st.setY(e.by[s] + d[j * 2 + 1]);
			}
		}
	}
}

// ---- parts
//: a named clip of one of the stage's planes, as the code reaches it (`getBackground().torches`):
//: every piece drawn under that name (ELEMENTS: each piece's index, the names over it, the labels it
//: plays and its sections), shown, hidden, faded and sent to a label or a frame together. a sectioned
//: piece plays the section a label names; the clip's own picture goes to a frame
var stageEls = ELEMENTS;
var partsMade = new haxe.ds.StringMap();
// a label as the pieces play it (only its letters and digits)
function phaseOf(l) {
	var s = "";
	var t = Std.string(l);
	for (i in 0...t.length) {
		var k = t.charCodeAt(i);
		if ((k >= 48 && k <= 57) || (k >= 65 && k <= 90) || (k >= 97 && k <= 122)) s += t.charAt(i);
	}
	return s;
}
// a sectioned piece sent to a label or a source frame
function goPiece(e, v, go) {
	var sec = e.sections;
	if (sec == null) { if (v.exports != null) v.exports.__go = go; return; }
	var k = -1;
	var at = 0;
	if (Std.isOfType(go, Float) || Std.isOfType(go, Int)) {
		var f = (Std.int(go) - 1) * 2;
		var start = 0;
		for (i in 0...sec.lens.length) {
			if (f >= start && f < start + sec.lens[i]) { k = i; at = f - start; }
			start += sec.lens[i];
		}
	} else k = e.labels.indexOf(go);
	if (k < 0) return;
	v.setVisible(true);
	v.playAnimation(sec.anims[k]);
	if (at > 0) v.playFrame(at + 1);
}
function partNamed(n) {
	if (n == null) return null;
	var have = partsMade.get(n);
	if (have != null) return have;
	var els = [];
	var depth = -1;
	for (e in stageEls) {
		var k = e.path.indexOf(n);
		if (k < 0) continue;
		els.push(e);
		if (depth < 0 || k < depth) depth = k;
	}
	var sf = SURFACES_ON ? surfaceNamed(n) : null;
	// (nothing drawn under the name: an object filed under it speaks for it, else it is a clip that
	// shows nothing, whose frame stays put)
	var obj = els.length == 0 && sf == null ? objectNamed(n) : null;
	if (obj != null && (obj.exports == null || obj.getCurrentFrame == null)) obj = null;
	var p = { name: n, shown: true, alpha: 1.0, fall: false };
	var each = function(f) { for (e in els) { var v = bootBg[e.i]; if (v != null) f(e, v); } };
	// (the clip's own picture: the pieces drawn right under it)
	var own = function(e) { return e.path.length == depth + 1; };
	p.setVisible = function(on) { p.shown = on == true; if (obj != null) obj.setVisible(p.shown); each(function(e, v) { v.setVisible(p.shown); }); };
	p.getVisible = function() { return p.shown; };
	p.setAlpha = function(a) { p.alpha = a; each(function(e, v) { if (v.setAlpha != null) v.setAlpha(a); }); };
	p.getAlpha = function() { return p.alpha; };
	p.playLabel = function(f, stop) {
		if (obj != null) { if (obj.exports.__playFrame != null) obj.exports.__playFrame(f); return; }
		if (!(Std.isOfType(f, Float) || Std.isOfType(f, Int))) {
			var l = phaseOf(f);
			each(function(e, v) { if (e.seg && e.labels.indexOf(l) >= 0) goPiece(e, v, l); });
			return;
		}
		var n2 = Std.int(f) < 1 ? 1 : Std.int(f);
		each(function(e, v) {
			if (!own(e)) return;
			if (e.seg) goPiece(e, v, n2);
			else if (v.playFrame != null) { v.playFrame(n2 * 2 - 1); if (stop) v.pause(); else v.resume(); }
		});
	};
	p.hold = function(stop) { each(function(e, v) { if (!own(e)) return; if (stop) { if (v.pause != null) v.pause(); } else if (v.resume != null) v.resume(); }); };
	// the frame its clip is on, as the source counts it: its first sectioned piece's section and frame
	p.getFrame = function() {
		if (obj != null) return Std.int((obj.getCurrentFrame() - 1) / 2) + 1;
		for (e in els) {
			var v = bootBg[e.i];
			if (v == null || v.getCurrentFrame == null) continue;
			var at = Std.int((v.getCurrentFrame() - 1) / 2);
			if (e.sections == null) return at + 1;
			var start = 0;
			for (i in 0...e.sections.anims.length) {
				if (v.getAnimation() == e.sections.anims[i]) return Std.int(start / 2) + at + 1;
				start += e.sections.lens[i];
			}
			return at + 1;
		}
		return 1;
	};
	p.getLabel = function() {
		for (e in els) {
			var v = bootBg[e.i];
			if (v == null || e.sections == null || v.getAnimation == null) continue;
			var k = e.sections.anims.indexOf(v.getAnimation());
			if (k >= 0) return e.labels[k];
		}
		return null;
	};
	// a surface placed under the name lets fighters through as the code says
	p.setFallthrough = function(on) { p.fall = on == true; if (sf != null) sf.updateStructureStats({ dropThrough: p.fall }); };
	p.getFallthrough = function() { return p.fall; };
	partsMade.set(n, p);
	return p;
}

// ---- timeline
//: a clip of the planes whose own frame code runs as its timeline reaches each frame (PART: the name
//: it is placed under; its cases are each frame's code)
var NAMEAt = 0;
function NAME() {
	var p = planePart(PART);
	if (p == null) return;
	var f = p.getFrame();
	if (f == NAMEAt) return;
	NAMEAt = f;
	switch (f) {
		// {cases}
	}
}
