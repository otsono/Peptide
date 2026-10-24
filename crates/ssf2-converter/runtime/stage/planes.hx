// A stage's planes: the clips its code reaches as its background, midground, foreground and
// shadow mask, which are the named children of the clip that is the whole stage, on their own
// timelines (`DATA.timelines`). What is drawn for them is what the stage pipeline made: one
// element per piece of art (`DATA.stage.elements`, each by the names over it and the labels it
// plays). A clip the code changes hands the change to the elements drawn under it: shown or
// hidden, faded, moved, or sent to a label or a frame. An element no code sends anywhere keeps
// the engine's own loop.
//
// Here too: the named surfaces the stage entity carries, the pieces with a ground friction or a
// sideways push of their own, surfaces riding a picture, and the clips the stage's code makes
// from its library.

var Planes = {};
// the stage clip's children the planes are, by the plane's name
Planes.childOf = function(p) {
	switch (p) {
		case "midground": return "terrain";
		default: return p;
	}
};
// a plane the stage clip does not place: a clip that answers like one and draws nothing
Planes.blank = new haxe.ds.StringMap();
Planes.get = function(p) {
	var root = PeptideRuntime.root;
	Planes.trackAll();
	var c = root == null ? null : root.getChildByName(Planes.childOf(p));
	if (c != null) return c;
	var b = Planes.blank.get(p);
	if (b == null) { b = MovieClip.create(); b.name = p; b.__auto = false; Planes.blank.set(p, b); }
	return b;
};

// a label as the elements play it (only its letters and digits)
Planes.phase = function(l) {
	var s = "";
	var t = Std.string(l);
	for (i in 0...t.length) {
		var k = t.charCodeAt(i);
		if ((k >= 48 && k <= 57) || (k >= 65 && k <= 90) || (k >= 97 && k <= 122)) s += t.charAt(i);
	}
	return s;
};
Planes.isNum = function(v) { return Std.isOfType(v, Float) || Std.isOfType(v, Int); };

// ---- binding: the named clips, and the elements drawn under each --------------------------------

// each named clip under the stage clip: its names from the stage clip down, where it stood when it
// was bound, and the elements drawn under it
Planes.bound = [];
// each element drawn under a named clip: its handle, where it was put, the labels it plays, and
// the clips over it
Planes.els = [];
// the clip whose own frame script is running (a clip's jump from its own timeline is one its
// elements already play)
Planes.framing = null;
// whether the named clips are followed yet, and whether their elements are made (a jump made
// before that, in the stage's initialize, waits for them)
Planes.tracked = false;
Planes.ready = false;

// the clip's own frame scripts are known as they run
Planes.install = function() {
	var run = MovieClip.runFrame;
	MovieClip.runFrame = function(me, n) {
		var was = Planes.framing;
		Planes.framing = me;
		run(me, n);
		Planes.framing = was;
	};
	PeptideAPI.clipByLinkage = Planes.libClip;
};

// the named clips are followed from the first time the code asks for a plane, or the boot
Planes.trackAll = function() {
	var root = PeptideRuntime.root;
	if (Planes.tracked || root == null) return;
	Planes.tracked = true;
	Planes.walk(root, []);
};

Planes.bind = function() {
	var root = PeptideRuntime.root;
	if (root == null) return;
	Planes.trackAll();
	// an element's names start at the stage clip's own placement: they are read from the first
	// name the stage clip places
	var top = [];
	for (k in root.__kids) if (k.clip != null && k.clip.__auto != true) top.push(k.clip.name);
	var rows = PeptideRuntime.data.stage.elements;
	for (e in rows) {
		var v = StageBoot.bg[e.i];
		if (v == null) continue;
		var path = e.path.copy();
		while (path.length > 0 && top.indexOf(path[0]) < 0) path.shift();
		var row = { v: v, seg: DATA.stage.boot.elements[e.i].seg == true, path: path, labels: e.labels, over: [],
		            bx: v.getX(), by: v.getY(), dx: 0.0, dy: 0.0, shown: true, alpha: 1.0 };
		for (b in Planes.bound) if (Planes.under(b.path, path)) {
			row.over.push(b);
			b.els.push(row);
			// as the clips over it stood when they were first followed: what the stage pipeline
			// drew it as
			if (b.v0 == false) row.shown = false;
			row.alpha *= b.a0;
		}
		if (row.over.length > 0) Planes.els.push(row);
	}
	// what the code asked of its clips before there was anything drawn for them
	Planes.ready = true;
	for (b in Planes.bound) for (a in b.waiting) {
		if (a.jump) Planes.jumped(b, a.f, a.stop); else Planes.held(b, a.stop);
	}
};

Planes.walk = function(c, path) {
	for (k in c.__kids) {
		var kid = k.clip;
		if (kid == null) continue;
		var p = path;
		if (kid.__auto != true) {
			p = path.copy();
			p.push(kid.name);
			Planes.track(kid, p);
		}
		if (kid.__kids != null) Planes.walk(kid, p);
	}
};

// whether the names `a` lead down to the names `b`
Planes.under = function(a, b) {
	if (a.length > b.length) return false;
	for (i in 0...a.length) if (a[i] != b[i]) return false;
	return true;
};

// a named clip is followed: where it stands now is its start, and a jump the code makes on it
// goes to its elements
Planes.track = function(c, path) {
	// (its place is counted off where its frame puts it: the elements drawn under it already move
	// as the frames move it, and only what the code does is left to hand them)
	var b = { c: c, path: path, els: [], waiting: [], x0: MovieClip.movedX(c), y0: MovieClip.movedY(c), v0: c.visible != false, a0: AS3.num(c.alpha),
	          lx: c.x, ly: c.y, lv: c.visible, la: c.alpha, dirty: false };
	Planes.bound.push(b);
	var gp = c.gotoAndPlay;
	var gs = c.gotoAndStop;
	var pl = c.play;
	var st = c.stop;
	c.gotoAndPlay = function(f, ?scene) { gp(f, scene); Planes.jumped(b, f, false); };
	c.gotoAndStop = function(f, ?scene) { gs(f, scene); Planes.jumped(b, f, true); };
	c.play = function() { pl(); Planes.held(b, false); };
	c.stop = function() { st(); Planes.held(b, true); };
	// a clip placed under the name of a surface the stage entity carries is that surface's: its
	// `fallthrough`, written by the code, switches the surface (castle siege lets each phase's
	// platforms through by its clip)
	if (c.__auto != true) for (r in PeptideRuntime.data.stage.surfaces) if (r.n == c.name) {
		var n = c.name;
		c.get_fallthrough = function() { return c.__fall == true; };
		c.set_fallthrough = function(v) {
			c.__fall = AS3.truthy(v);
			var sf = Planes.surface(n);
			if (sf != null) PeptidePlatform.setFallthrough(sf, c.__fall);
			return v;
		};
		break;
	}
};

// the code sent a clip to a label or a frame: every element under it that plays that label goes
// there, and one that is the clip's own picture goes to the frame
Planes.jumped = function(b, f, stop) {
	// the clip's own frame script sent it to a frame: a picture drawn as the clip's plain loop does
	// not know that jump (the art is the timeline as it stands), so it goes there too and plays on,
	// as a clip under a plane does (Stages.playOn): krazoa palace's fan blades go back to frame 1
	// from 15 and never show their still half. a sectioned element plays its own sections already
	if (Planes.framing == b.c) {
		if (!Planes.ready || !Planes.isNum(f)) return;
		var at = Std.int(f) < 1 ? 1 : Std.int(f);
		for (e in b.els) if (!e.seg && e.path.length == b.path.length && e.v.playFrame != null) { e.v.playFrame(at * 2 - 1); e.v.resume(); }
		return;
	}
	if (!Planes.ready) { b.waiting.push({ jump: true, f: f, stop: stop }); return; }
	if (!Planes.isNum(f)) {
		var l = Planes.phase(f);
		for (e in b.els) if (e.labels.indexOf(l) >= 0) {
			if (e.seg && e.v.exports != null) e.v.exports.__go = l;
		}
		for (p in PeptideRuntime.data.stage.placedAt) if (p.l == Std.string(f)) {
			var o = __objectNamed(p.n);
			if (o != null && o.setX != null) { o.setX(p.x); o.setY(p.y); }
		}
		return;
	}
	var n = Std.int(f);
	if (n < 1) n = 1;
	for (e in b.els) if (Planes.drives(b, e)) {
		if (e.seg) { if (e.v.exports != null) e.v.exports.__go = n; }
		else {
			e.v.playFrame(n * 2 - 1);
			if (stop) e.v.pause(); else e.v.resume();
			e.held = stop;
		}
	}
};

// whether the code holds this element on a frame of its choosing (the verify gates read it)
Planes.holds = function(v) {
	for (e in Planes.els) if (e.v == v) return e.held == true;
	return false;
};

// whether the clip's frame is what an element under it shows: it is the clip's own picture, or
// a piece that only rides the clip's timeline (every clip between them holds one frame, and the
// element's loop is the clip's length on this engine's clock)
Planes.drives = function(b, e) {
	if (e.path.length == b.path.length) return true;
	if (e.seg || e.v.getTotalFrames == null || e.v.getTotalFrames() != AS3.num(b.c.totalFrames) * 2) return false;
	var c = b.c;
	for (i in b.path.length...e.path.length) {
		c = c.getChildByName(e.path[i]);
		if (c == null || AS3.num(c.totalFrames) > 1) return false;
	}
	return true;
};

// the code stopped a clip, or played it on: its own pictures hold or go on
Planes.held = function(b, stop) {
	if (Planes.framing == b.c) return;
	if (!Planes.ready) { b.waiting.push({ jump: false, f: null, stop: stop }); return; }
	for (e in b.els) if (Planes.drives(b, e)) {
		if (stop) { if (e.v.pause != null) e.v.pause(); }
		else if (e.v.resume != null) e.v.resume();
		e.held = stop;
	}
};

// every frame: what the code changed on a clip since (shown, faded, moved) reaches its elements
Planes.frame = function() {
	var s = PeptideRuntime.data.units.size;
	var dirty = false;
	for (b in Planes.bound) {
		var c = b.c;
		// the clip came into another labelled section by playing on, or its own frame script sent
		// it there (a plane plays its transition out into `middle`): its segmented elements go to
		// that section, as a jump by the code sends them
		var lab = c.currentLabel;
		if (lab != b.lab) {
			var was = b.lab;
			b.lab = lab;
			if (was != null && lab != null && Planes.ready) {
				var l = Planes.phase(lab);
				for (e in b.els) if (e.seg && e.labels.indexOf(l) >= 0 && e.v.exports != null && e.v.exports.__go != l) e.v.exports.__go = l;
			}
		}
		if (c.x != b.lx || c.y != b.ly || c.visible != b.lv || c.alpha != b.la) {
			b.lx = c.x; b.ly = c.y; b.lv = c.visible; b.la = c.alpha;
			b.dirty = true;
			dirty = true;
		}
	}
	if (!dirty) return;
	for (e in Planes.els) {
		var touched = false;
		for (b in e.over) if (b.dirty == true) touched = true;
		if (!touched) continue;
		var dx = 0.0;
		var dy = 0.0;
		var shown = true;
		var alpha = 1.0;
		for (b in e.over) {
			dx += MovieClip.movedX(b.c) - b.x0;
			dy += MovieClip.movedY(b.c) - b.y0;
			if (b.c.visible == false) shown = false;
			alpha *= AS3.num(b.c.alpha);
		}
		if (dx != e.dx || dy != e.dy) {
			e.dx = dx;
			e.dy = dy;
			e.v.setX(e.bx + dx * s);
			e.v.setY(e.by + dy * s);
		}
		if (shown != e.shown) { e.shown = shown; e.v.setVisible(shown); }
		if (alpha != e.alpha) { e.alpha = alpha; e.v.setAlpha(alpha); }
	}
	for (b in Planes.bound) b.dirty = false;
};

// the camera backgrounds' clips the stage holds: shown, hidden and faded as the code sets them
Planes.camBgs = function() {
	var me = Stage.current;
	if (me == null || me.__camBgs == null) return;
	var cbg = match.getCamera() == null ? null : match.getCamera().getBackgroundContainers();
	if (cbg == null) return;
	for (i in 0...me.__camBgs.length) {
		if (i >= cbg.length) break;
		var mc = me.__camBgs[i].mc;
		// as the stage first holds it is as the engine draws it; only a change goes across
		if (mc.__seen != true) { mc.__seen = true; mc.__lv = mc.visible; mc.__la = mc.alpha; continue; }
		if (mc.visible != mc.__lv) { mc.__lv = mc.visible; cbg[i].visible = mc.visible != false; }
		if (mc.alpha != mc.__la) { mc.__la = mc.alpha; cbg[i].alpha = AS3.num(mc.alpha); }
	}
};

// ---- surfaces ---------------------------------------------------------------------------------

// a surface the stage entity carries under a name: its structures, moved and switched together,
// standing where the tree placed its clip
Planes.surfaces = new haxe.ds.StringMap();
Planes.surface = function(n) {
	var have = Planes.surfaces.get(n);
	if (have != null) return have;
	var row = null;
	for (r in PeptideRuntime.data.stage.surfaces) if (r.n == n) row = r;
	if (row == null) return null;
	// its structures, found once the stage has them
	var sts = [];
	var find = function() {
		if (sts.length == 0) for (k in row.s) { var st = match.getStructureByName(k); if (st != null) sts.push(st); }
		return sts;
	};
	var at = null;
	for (r in PeptideRuntime.data.stage.regions) if (r.n == n) at = r;
	var e = { x: at == null ? 0.0 : at.rx, y: at == null ? 0.0 : at.ry, exports: {} };
	e.getX = function() { return e.x; };
	e.getY = function() { return e.y; };
	e.setX = function(v) { var d = v - e.x; e.x = v; for (st in find()) st.setX(st.getX() + d); };
	e.setY = function(v) { var d = v - e.y; e.y = v; for (st in find()) st.setY(st.getY() + d); };
	e.isDisposed = function() { return false; };
	e.updateStructureStats = function(o) { for (st in find()) st.updateStructureStats(o); };
	e.getStructureStat = function(k) { var l = find(); return l.length > 0 ? l[0].getStructureStat(k) : null; };
	e.addToBlacklist = function(o) { for (st in find()) st.addToBlacklist(o); };
	e.removeFromBlacklist = function(o) { for (st in find()) st.removeFromBlacklist(o); };
	var me = {};
	PeptidePlatform.build(me);
	me.__fm = e;
	me.name = n;
	// a ground friction its code gives it as the match runs (an ice floor): a fighter standing on
	// it slows and speeds up by it, the way a piece that declares one does
	me.__structures = find;
	me.setDecelFriction = function(v) { me.__decel = AS3.num(v) * PeptideRuntime.data.units.accel; if (Planes.coded.indexOf(me) < 0) Planes.coded.push(me); };
	me.setAccelFriction = function(v) { me.__accel = AS3.num(v) * PeptideRuntime.data.units.accel; if (Planes.coded.indexOf(me) < 0) Planes.coded.push(me); };
	Planes.surfaces.set(n, me);
	return me;
};
Planes.coded = [];

// the pieces with a ground friction of their own (a fighter standing in one slows by it, and gets
// its own back on leaving) and the ones with a sideways push (a fighter standing in one is moved)
Planes.frictionHeld = [];
Planes.ground = function() {
	var fr = PeptideRuntime.data.stage.frictions;
	var pu = PeptideRuntime.data.stage.pushes;
	if (fr.length == 0 && pu.length == 0 && Planes.coded.length == 0) return;
	var ps = match.getCharacters();
	for (i in 0...ps.length) {
		var p = ps[i];
		while (Planes.frictionHeld.length <= i) Planes.frictionHeld.push(null);
		var on = null;
		if (p.isOnFloor()) {
			var x = p.getX();
			var y = p.getY();
			for (b in fr) if (x >= b.x0 && x <= b.x1 && y >= b.y0 && y <= b.y1) on = b;
			// a surface whose code set its friction, by the floor the fighter stands on
			if (Planes.coded.length > 0) {
				var floor = p.getCurrentFloor();
				for (c in Planes.coded) if (floor != null && c.__structures().indexOf(floor) >= 0) {
					var f0 = p.getCharacterStat("friction");
					var a0 = p.getCharacterStat("walkSpeedAcceleration");
					var held = Planes.frictionHeld[i];
					on = { f: c.__decel != null ? c.__decel : (held != null ? held.f : f0), a: c.__accel != null ? c.__accel : (held != null ? held.a : a0) };
				}
			}
			for (b in pu) if (x >= b.x0 && x <= b.x1 && y >= b.y0 && y <= b.y1) p.setX(p.getX() + b.dx);
		}
		if (on != null) {
			if (Planes.frictionHeld[i] == null) Planes.frictionHeld[i] = { f: p.getCharacterStat("friction"), a: p.getCharacterStat("walkSpeedAcceleration") };
			// (again when what is under it changes: a surface's code can change its own)
			if (p.getCharacterStat("friction") != on.f || p.getCharacterStat("walkSpeedAcceleration") != on.a)
				p.updateCharacterStats({ friction: on.f, walkSpeedAcceleration: on.a });
		} else if (Planes.frictionHeld[i] != null) {
			p.updateCharacterStats({ friction: Planes.frictionHeld[i].f, walkSpeedAcceleration: Planes.frictionHeld[i].a });
			Planes.frictionHeld[i] = null;
		}
	}
};

// a surface whose plane moves it by label rides its own picture: its structures sit where the
// picture's current frame has moved from where it was put
Planes.riding = function() {
	for (e in __platFollow) {
		if (e.v == null) continue;
		var l = (e.v.exports != null && e.v.exports.__label != null) ? e.v.exports.__label : e.v.getAnimation();
		var d = null;
		for (a in e.at) if (a.l == l) d = a.d;
		if (d == null || d.length == 0) continue;
		var j = Std.int((e.v.getCurrentFrame() - 1) / 2);
		if (j < 0) j = 0;
		if (j * 2 + 1 >= d.length) j = Std.int(d.length / 2) - 1;
		for (r in PeptideRuntime.data.stage.surfaces) if (r.n == e.n) {
			for (s in 0...r.s.length) {
				var st = match.getStructureByName(r.s[s]);
				if (st == null) continue;
				if (e.bx.length <= s) { e.bx.push(st.getX()); e.by.push(st.getY()); }
				st.setX(e.bx[s] + d[j * 2]);
				st.setY(e.by[s] + d[j * 2 + 1]);
			}
		}
	}
};

// ---- clips the code makes from the library ----------------------------------------------------

// each one made: the clip, and the effects drawn for its pieces (made once it is in a plane)
Planes.libs = [];
Planes.libClip = function(n) {
	var row = null;
	for (r in PeptideRuntime.data.stage.lib) if (r.n == AS3.str(n)) row = r;
	if (row == null) return MovieClip.create();
	var c = row.sym != null ? PeptideRuntime.makeClip(row.sym) : MovieClip.create();
	Planes.libs.push({ c: c, parts: row.parts, vs: [] });
	return c;
};
// the plane a clip is in (the stage clip's child it is under), or null when it is in none
Planes.planeOf = function(c) {
	var root = PeptideRuntime.root;
	var p = c;
	var n = 0;
	while (p != null && p.parent != root && n < 64) { p = p.parent; n++; }
	return (p != null && p.parent == root) ? p.name : null;
};
// every frame: a made clip in a plane is drawn where it is, as it is; one taken out goes
Planes.libFrame = function() {
	if (Planes.libs.length == 0) return;
	var s = PeptideRuntime.data.units.size;
	var o = PeptideRuntime.data.stage.origin;
	var keep = [];
	for (l in Planes.libs) {
		var plane = Planes.planeOf(l.c);
		if (plane == null) {
			if (l.vs.length > 0) { for (v in l.vs) { m_owned.remove(v.v); v.v.destroy(); } l.vs = []; }
			if (l.c.parent == null && l.made == true) continue;
			keep.push(l);
			continue;
		}
		if (l.vs.length == 0) {
			l.made = true;
			var into = plane == "foreground" ? self.getForegroundStructuresContainer() : (plane == "terrain" ? self.getBackgroundContainer() : self.getBackgroundBehindContainer());
			for (p in l.parts) {
				var v = match.createVfx(new VfxStats({ spriteContent: self.getResource().getContent(p.id), animation: "active", x: 0, y: 0, loop: true, timeout: -1, relativeWith: false, resizeWith: false }));
				if (v == null) continue;
				into.addChild(v.getViewRootContainer());
				m_owned.push(v);
				l.vs.push({ v: v, ax: p.ax, ay: p.ay, w: p.w });
			}
		}
		// where it is in the stage's space: its place and its parents', each scaled by the ones over it
		var x = 0.0;
		var y = 0.0;
		var sx = 1.0;
		var sy = 1.0;
		var shown = true;
		var alpha = 1.0;
		var c = l.c;
		var n = 0;
		while (c != null && c != PeptideRuntime.root && n < 64) {
			x = x * AS3.num(c.scaleX) + AS3.num(c.x);
			y = y * AS3.num(c.scaleY) + AS3.num(c.y);
			sx *= AS3.num(c.scaleX);
			sy *= AS3.num(c.scaleY);
			if (c.visible == false) shown = false;
			alpha *= AS3.num(c.alpha);
			c = c.parent;
			n++;
		}
		for (e in l.vs) {
			e.v.setX((x - o.x) * s + (sx < 0 ? -(e.ax + e.w) : e.ax));
			e.v.setY((y - o.y) * s + e.ay);
			e.v.setScaleX(sx);
			e.v.setScaleY(sy);
			e.v.setVisible(shown);
			e.v.setAlpha(alpha);
		}
		keep.push(l);
	}
	Planes.libs = keep;
};
