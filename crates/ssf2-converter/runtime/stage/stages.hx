// A stage, as the source's engine runs one: the class its package registers as its stage, on the
// clip that is the whole stage, with what the conversion worked out around it as data
// (`DATA.stage`): the boot (made by `StageBoot`), the regions its code asks for by name, and the
// camera backgrounds it declares. Here is the stage's upkeep: the boot once the match is live, the
// names things are filed under, and the schedules and followers the boot put down.

// what the boot fills and the stage's frame keeps: what it owns (gone with the stage), containers
// the stage's timeline shows and hides, colour and opacity schedules, pieces riding an object,
// surfaces riding a picture
var m_owned = [];
var m_shown = [];
var m_tint = [];
var m_alpha = [];
var __follow = [];
var __platFollow = [];

// ---- names: what the stage's code asks things by ----------------------------------------------

var __namedObjs = [];
function __nameObject(n, o) {
	if (o != null) __namedObjs.push({ n: n, o: o });
}
// what is filed under a name: an object the boot made, else a region the tree placed under it
function __objectNamed(n) {
	for (e in __namedObjs) if (e.n == n) return e.o;
	return Stages.region(n);
}

// a named clip that came out as pieces: the pieces, shown, hidden and faded together
function __clipGroup(parts, labels, name, ws, plane) {
	var g = { parts: parts, labels: labels, name: name, widths: ws, plane: plane, visible: true };
	g.setVisible = function(v) {
		g.visible = v;
		for (p in parts) if (p != null) p.setVisible(v);
	};
	g.setAlpha = function(a) {
		for (p in parts) if (p != null) p.setAlpha(a);
	};
	return g;
}

// one container per pan rate, moved against the camera by that rate: a piece at 0 stays with the
// view, one at 1 stays in the world
var __panRates = [];
var __panConts = [];
function __panContainer(px, py) {
	for (i in 0...__panRates.length) if (__panRates[i].x == px && __panRates[i].y == py) return __panConts[i];
	var c = Container.create();
	self.getBackgroundBehindContainer().addChild(c);
	__panRates.push({ x: px, y: py });
	__panConts.push(c);
	return c;
}

var Stages = {};
// the engine class a stage is, whatever class its package declares for it
Stages.api = function() { return PeptideStage; };
Stages.entering = function(me, anim, ?again) {};
// its animation runs on as the engine plays it
Stages.playsOut = true;
// where it comes in the source engine's pass over its objects: first
Stages.order = 0;
// the stage's own code runs from when the match is live: until there are fighters to put down the
// boot for, nothing of the stage moves on (its timers and schedules start with the match)
Stages.waits = function(me) {
	if (__verifyBuilt != true) Stages.boot(me);
	return __verifyBuilt != true;
};
// a plane the stage's code sends to a frame plays on from it, as the source's does, until one of its
// own frame scripts stops it (a `stop()`, or a `gotoAndStop` back to where it holds); then it holds
// until the code sends it again (measured: castle siege's background plays its transition out after
// the code's `gotoAndStop("transition1")`, stops at the frame that stops it, and plays the next part
// out from `transition1_end`). a plane its code never sends anywhere is left as its timeline has it
Stages.planeNames = ["background", "foreground", "midground", "shadowMask"];
Stages.beforeTick = function(me) {
	var root = PeptideRuntime.root;
	if (root == null) return;
	for (n in Stages.planeNames) {
		var c = root.getChildByName(Planes.childOf(n));
		if (c == null || c.__tl == null) continue;
		if (c.__playsOnWrapped != true) {
			c.__playsOnWrapped = true;
			var stopAt = c.gotoAndStop;
			var halt = c.stop;
			// sent by the code: plays on (set first, so a stop in the frame it lands on still holds it)
			c.gotoAndStop = function(f, ?scene) {
				if (Planes.framing != c) { c.__playsOn = true; stopAt(f, scene); }
				else { c.__playsOn = false; stopAt(f, scene); }
			};
			c.stop = function() { if (Planes.framing == c) c.__playsOn = false; halt(); };
		}
		if (c.__playsOn == true) c.__playing = true;
	}
	// a named clip under a plane plays on whatever its own frame scripts stop: krazoa palace's fan
	// blades send themselves back to frame 1 with `gotoAndStop(1)` and keep turning in the source
	// (measured with tools/tests/stage_watch.py: 2, 4 .. 14, 2 ..), where a plane itself holds where
	// its own scripts stop it (castle siege's background). only the clips the planes follow, which
	// are few: every clip of a plane every frame was a stage's whole frame
	for (b in Planes.bound) if (b.c.totalFrames > 1 && Stages.isPlane(b.c) == false) b.c.__playing = true;
};
Stages.isPlane = function(c) {
	var root = PeptideRuntime.root;
	for (n in Stages.planeNames) if (root.getChildByName(Planes.childOf(n)) == c) return true;
	return false;
};
Stages.frozen = function(me) { return false; };
// the named clips the boot made, by name
Stages.groups = new haxe.ds.StringMap();
// how many frames the stage has run
Stages.frames = 0;
Stages.panX0 = null;
Stages.panY0 = null;

// the stage's engine object as the runtime holds it. the runtime asks every object it runs for its
// speeds and its owner around the code it runs, and whether it is gone; a stage has no speeds and
// no owner and lasts the match, so those are answered here. the rest is the stage's own, asked only
// of a stage that answers it (one that plays no animation of its own shows its one frame)
Stages.engine = function(fm) {
	var e = { exports: fm.exports, stage: fm };
	e.getXSpeed = function() { return 0; };
	e.getYSpeed = function() { return 0; };
	e.getOwner = function() { return null; };
	e.isDisposed = function() { return false; };
	e.getX = function() { return fm.getX(); };
	e.getY = function() { return fm.getY(); };
	e.getResource = function() { return fm.getResource(); };
	e.getAnimation = function() { return fm.getAnimation != null ? fm.getAnimation() : null; };
	e.getCurrentFrame = function() { return fm.getCurrentFrame != null ? fm.getCurrentFrame() : 1; };
	e.getTotalFrames = function() { return fm.getTotalFrames != null ? fm.getTotalFrames() : 1; };
	e.playAnimation = function(a) { if (fm.playAnimation != null) fm.playAnimation(a); };
	e.playFrame = function(n) { if (fm.playFrame != null) fm.playFrame(n); };
	e.pause = function() { if (fm.pause != null) fm.pause(); };
	e.resume = function() { if (fm.resume != null) fm.resume(); };
	return e;
};

// as the stage is made: it is the stage every script's code asks for, and its api calls go to the
// stage itself. the bodies the tree placed are there before its own code runs, which asks for them
// by name
Stages.begin = function(me) {
	me.__fm = PeptideRuntime.fm.stage;
	Stage.current = me;
	// its planes, for the objects' scripts that ask the stage for one (they hold no clip tree)
	me.__planeOf = function(p) { return Planes.get(p); };
	StageAPI.install();
	Planes.install();
	StageObjects.install();
	Stages.objects();
};
// the bodies the tree placed, once the stage's clip is there: they are part of the stage from its
// first frame, ahead of the fighters and of the stage's own code
Stages.made = function(me) { StageBoot.placed(null); };

// the objects the stage puts down are its classes' objects: one the stage's own code makes it
// leaves to that code
Stages.objects = function() {
	StageObjects.stageExports = PeptideRuntime.fm.exports;
	// what the stage files under a name, for its objects' code too
	PeptideRuntime.fm.exports.__named = function(n) { return Stages.named(n); };
	// ...and its planes, for the objects' scripts that ask the stage for one
	PeptideRuntime.fm.exports.__planeOf = function(p) { return Planes.get(p); };
	var byCode = PeptideRuntime.data.stage.byCode;
	var put = StageBoot.object;
	StageBoot.object = function(o, owner) {
		if (byCode.indexOf(o.id) >= 0) return;
		put(o, owner);
		var h = StageBoot.hz[o.i];
		var n = PeptideRuntime.data.contentClass.get(o.id);
		if (h != null && n != null) StageObjects.view(h, n);
	};
};

// a plane the stage's code asks for: the stage clip's own child (planes.hx)
Stages.planeClip = function(p) { return Planes.get(p); };

// every frame, ahead of the stage's own code (the boot came first, in `waits`)
Stages.frame = function(me) {};

// the stage's own code starts with the match, as the source's does: its players are there to ask for
Stages.initLate = true;

// what the stage puts down once there are fighters: the match's hazards switch decides what the
// source put behind it. then the stage's own code starts
Stages.boot = function(me) {
	var chars = match.getCharacters();
	if (chars == null || chars.length == 0) return;
	var cfg = match.getMatchSettingsConfig();
	var b = PeptideRuntime.data.stage.boot;
	StageBoot.spawn(null, chars, cfg == null || cfg.hazards);
	for (k in 0...b.groups.length) Stages.groups.set(b.groups[k].n, StageBoot.group(k));
	if (b.camBg.length > 0) StageBoot.camBg();
	Planes.bind();
	__verifyBuilt = true;
	if (me != null) PeptideRuntime.run(me.initialize);
};

// the stage's clock: its animation's frame while it plays one longer than a frame, else the frames
// it has run
Stages.clock = function() {
	var f = PeptideRuntime.fm.getCurrentFrame();
	return f > 1 ? f : Stages.frames;
};

// every one of this engine's frames: what rides an object follows it, and the schedules move on
Stages.engineFrame = function(me) {
	Stages.frames++;
	for (f in __follow) {
		if (f.v == null || f.o == null) continue;
		f.v.setX(f.o.getX() + (f.dx != null ? f.dx : 0));
		f.v.setY(f.o.getY() + (f.dy != null ? f.dy : 0));
	}
	var t = Stages.clock();
	// shown over runs of [from, to) on the clock: a cursor per container, moved on only forward
	for (e in m_shown) {
		if (t < e.at) e.i = 0;
		e.at = t;
		while (e.i < e.on.length && t >= e.on[e.i][1]) e.i++;
		e.c.visible = e.i < e.on.length && t >= e.on[e.i][0];
	}
	Stages.pan();
	// what the code changed on its clips reaches what is drawn for them, and the surfaces and the
	// weather move on
	Planes.frame();
	Planes.camBgs();
	Planes.ground();
	Planes.riding();
	Planes.libFrame();
	Weather.frame();
	// opacity and colour hold each key until the next, on the piece's own loop
	for (e in m_alpha) {
		var c = e.len > 0 ? t % e.len : t;
		Stages.key(e, c);
		e.c.alpha = e.k[e.i][1];
	}
	for (e in m_tint) {
		var c = e.len > 0 ? t % e.len : t;
		Stages.key(e, c);
		var k = e.k[e.i];
		e.s.redMultiplier = k[1];
		e.s.greenMultiplier = k[2];
		e.s.blueMultiplier = k[3];
		e.s.redOffset = k[4];
		e.s.greenOffset = k[5];
		e.s.blueOffset = k[6];
	}
};
Stages.key = function(e, c) {
	if (c < e.at) e.i = 0;
	e.at = c;
	while (e.i + 1 < e.k.length && c >= e.k[e.i + 1][0]) e.i++;
};

// the pan containers, against where the camera started (where it is now is a place in the world)
Stages.pan = function() {
	if (__panConts.length == 0) return;
	var cam = match.getCamera();
	if (cam == null) return;
	if (Stages.panX0 == null) { Stages.panX0 = cam.getX(); Stages.panY0 = cam.getY(); }
	var dx = cam.getX() - Stages.panX0;
	var dy = cam.getY() - Stages.panY0;
	var base = PeptideRuntime.data.stage.pan;
	for (i in 0...__panConts.length) {
		__panConts[i].x = cam.getX() - base.x * 0.5 - dx * __panRates[i].x;
		__panConts[i].y = cam.getY() - base.y * 0.5 - dy * __panRates[i].y;
	}
};

// a region the tree placed under a name, as the source reads one: where its clip is placed, and the
// rectangle it covers, in the source's units
Stages.region = function(n) {
	var s = PeptideRuntime.data.units.size;
	for (r in PeptideRuntime.data.stage.regions) {
		if (r.n != n) continue;
		return { name: n, x: r.rx / s, y: r.ry / s, width: r.w / s, height: r.h / s,
		         rect: { x: r.x / s, y: r.y / s, width: r.w / s, height: r.h / s } };
	}
	return null;
};

// what the stage files under a name, as the source's api hands it back: an object it made, a
// surface the stage entity carries, or a region its tree placed
Stages.named = function(n) {
	for (e in __namedObjs) if (e.n == n) {
		for (st in PeptideRuntime.data.stage.boot.structures) if (st.name == n) return Stages.withClip(Platforms.view(e.o), n);
		return Objects.view(e.o);
	}
	var s = Planes.surface(n);
	if (s != null) return Stages.withClip(s, n);
	return Stages.boundary(n);
};
// a surface the tree placed under a name is that clip's: its clip is the one of that name, which
// the code shows and hides it by (hylian skies swaps its light and dark terrain), and what the
// planes draw under the clip follows it
Stages.withClip = function(p, n) {
	if (p == null || p.__mc != null) return p;
	Planes.trackAll();
	for (b in Planes.bound) if (b.path.length > 0 && b.path[b.path.length - 1] == n) { p.__mc = b.c; break; }
	return p;
};

// a region the tree placed, as the source's code holds one: the point its clip is placed by, and
// the rectangle it covers, moved together when the code moves it (this engine's units in)
Stages.boundaries = new haxe.ds.StringMap();
Stages.boundary = function(n) {
	var have = Stages.boundaries.get(n);
	if (have != null) return have;
	var row = null;
	for (r in PeptideRuntime.data.stage.regions) if (r.n == n) row = r;
	if (row == null) return null;
	var rect = { x: row.x, y: row.y, w: row.w, h: row.h };
	var e = { x: row.rx, y: row.ry };
	e.getX = function() { return e.x; };
	e.getY = function() { return e.y; };
	e.setX = function(v) { rect.x += v - e.x; e.x = v; };
	e.setY = function(v) { rect.y += v - e.y; e.y = v; };
	e.isDisposed = function() { return false; };
	var me = {};
	PeptideCollisionBoundary.build(me);
	me.__fm = e;
	me.__rect = rect;
	me.name = n;
	Stages.boundaries.set(n, me);
	return me;
};

// the stage ends: what it made for itself goes with it
Stages.teardown = function() {
	for (o in m_owned) if (o != null) o.destroy();
	m_owned = [];
};
