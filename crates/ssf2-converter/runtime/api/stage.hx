// The stage as the source's code sees it: its bounds, and the layer the fighters are drawn in.

// class PeptideStage extends PeptideBaseAPIObject
var PeptideStage = { name: "PeptideStage" };
PeptideStage.build = function(me) {
	PeptideBaseAPIObject.build(me);
	me.__is.push("PeptideStage");
	me.__type = "SSF2Stage";
};
PeptideStage.getCameraBounds = function(me) { return Stage.rect(me.__fm == null ? null : me.__fm.getCameraBounds()); };
PeptideStage.getDeathBounds = function(me) { return Stage.rect(me.__fm == null ? null : me.__fm.getDeathBounds()); };
// the HUD is this engine's: clips the source can hold and add to, never drawn
PeptideStage.hudBack = function(me) { if (me.__hudBack == null) me.__hudBack = MovieClip.create(); return me.__hudBack; };
PeptideStage.hudFront = function(me) { if (me.__hudFront == null) me.__hudFront = MovieClip.create(); return me.__hudFront; };
PeptideStage.init = function(me, ?api) {};
// the stage's death by its top and bottom bounds: the engine's bounds stay as they are
PeptideStage.enableFallDeath = function(me, on) { me.__fallDeath = AS3.truthy(on); PeptideRuntime.gap("enableFallDeath: the death bounds switched by the stage"); };
PeptideStage.enableCeilingDeath = function(me, on) { me.__ceilingDeath = AS3.truthy(on); PeptideRuntime.gap("enableCeilingDeath: the death bounds switched by the stage"); };
PeptideStage.isFallDeathEnabled = function(me) { return me.__fallDeath != false; };
PeptideStage.isCeilingDeathEnabled = function(me) { return me.__ceilingDeath != false; };
// the clip the stage's light is: the source's to hold and move. the engine's light stays where the
// conversion put it
PeptideStage.getLightSourceMC = function(me) {
	if (me.__light == null) {
		PeptideRuntime.gap("getLightSourceMC: the engine's light stays where the conversion put it");
		me.__light = MovieClip.create();
		me.__light.name = "light_source";
		me.__light.__auto = false;
	}
	return me.__light;
};
// the camera backgrounds the package declares, in its order, each as the source holds one (its
// clip under `mc`). the engine draws and pans them; the clips are the source's to hold and set
PeptideStage.getCameraBackgrounds = function(me) {
	if (me.__camBgs == null) {
		me.__camBgs = [];
		var st = PeptideRuntime.data == null ? null : PeptideRuntime.data.stage;
		if (st != null) for (i in 0...st.cameraBackgrounds.length) {
			var l = st.cameraBackgrounds[i];
			// its own clip, with what it holds, where the package has one
			var sym = st.cameraBackgroundClips == null ? null : st.cameraBackgroundClips[i];
			var mc = sym != null ? PeptideRuntime.makeClip(sym) : MovieClip.create();
			mc.name = l;
			mc.__auto = false;
			me.__camBgs.push({ mc: mc, linkage_id: l });
		}
	}
	return me.__camBgs;
};
// the clip the stage's weather is put in: drawn over the view, wherever the camera is
PeptideStage.getWeatherMC = function(me) { return Weather.clip(); };
// a function run every `ms` milliseconds until it is cleared: the source's frames are a thirtieth
// of a second, so every that many of them (at least one), on the stage's own timers
PeptideStage.setInterval = function(me, fn, ms) {
	var n = Math.round(AS3.num(ms) * 30 / 1000);
	if (n < 1) n = 1;
	Timers.create(me, n, 0, fn, null);
	return { fn: fn };
};
PeptideStage.clearInterval = function(me, h) { if (h != null && h.fn != null) Timers.destroy(me, h.fn); };
// what the source's engine places fighters and items by, and the smash ball's area: this engine
// keeps its own, so the stage gets places to hold and move that move nothing here
PeptideStage.getSpawnPositionMCs = function(me) { PeptideRuntime.gap("getSpawnPositionMCs: the engine's own spawn points"); return []; };
PeptideStage.getSmashBallBounds = function(me) { PeptideRuntime.gap("getSmashBallBounds: the smash ball's area"); return Rectangle.create(0, 0, 0, 0); };
PeptideStage.getItemSpawnBoundsList = function(me) { PeptideRuntime.gap("getItemSpawnBoundsList: where items fall"); return [Rectangle.create(0, 0, 0, 0)]; };

var Stage = {};
Stage.current = null;
// a plane of the stage, asked from any script: the stage's own, where its script hands them out,
// else the layer the fighters are drawn in
Stage.planeOf = function(st, p) {
	var s = st != null && st.__planeOf != null ? st : Stage.get();
	return s != null && s.__planeOf != null ? s.__planeOf(p) : Stage.layer;
};
// what asks the engine's line test against the structures: a game object's own, and asked by the
// stage (no game object), a fighter's, since the test reads every structure whoever asks
Stage.tester = function(fm) {
	if (fm != null && fm.hitTestStructuresWithLineSegment != null) return fm;
	var cs = match.getCharacters();
	return cs == null || cs.length == 0 ? null : cs[0];
};
Stage.get = function() {
	var fm = stage;
	if (Stage.current == null || Stage.current.__fm != fm) {
		var me = {};
		PeptideStage.build(me);
		me.__fm = fm;
		Stage.current = me;
	}
	// asked from another script: the stage's planes, as the stage's own script answers them (once
	// it has started)
	var c = Stage.current;
	if (c.__planeOf == null && fm != null && fm.exports != null && fm.exports.__planeOf != null) c.__planeOf = fm.exports.__planeOf;
	return c;
};

// an area of this engine's as the source keeps one: a rectangle in its units. an area's
// rectangle is its own, placed at the area's origin, so the world's is the two added
Stage.rect = function(a) {
	if (a == null) return null;
	var r = a.getRectangle();
	if (r == null) return null;
	var s = PeptideRuntime.data.units.size;
	var x = (a.getX() + r.x) / s;
	var y = (a.getY() + r.y) / s;
	var w = r.width / s;
	var h = r.height / s;
	return { x: x, y: y, width: w, height: h, left: x, top: y, right: x + w, bottom: y + h };
};

// the engine's object a display thing stands for: a fighter's clip is the fighter, an effect is
// the engine's own object already
Stage.engineOf = function(o) {
	if (o == null) return null;
	if (o.__vfx != null) return o.__vfx;
	if (o.__object != null) return o.__object.__fm;
	if (o.__mc != null && o.__fm != null) return o.__fm;
	if (o.__fm != null) return o.__fm;
	if (o.getViewRootContainer != null) return o;
	return null;
};

// the layer the fighters and their effects are drawn in. two game objects trade places by the
// engine's own swap; an effect has no place among them, so trading places with one moves the
// effect to the other side of the fighters (behind them, or back in front)
Stage.behind = [];
Stage.isEffect = function(o) { return o.getType() == EntityType.VFX; };
Stage.layer = {
	// placed at the world's origin: the source's positions here are already the world's
	x: 0.0, y: 0.0, scaleX: 1.0, scaleY: 1.0,
	swapChildren: function(a, b) {
		var x = Stage.engineOf(a);
		var y = Stage.engineOf(b);
		if (x == null || y == null || x == y) return;
		if (!Stage.isEffect(x) && !Stage.isEffect(y)) { x.swapDepths(y); return; }
		var v = Stage.isEffect(x) ? x : y;
		var i = Stage.behind.indexOf(v);
		if (i >= 0) {
			Stage.behind.splice(i, 1);
			stage.getCharactersFrontContainer().addChild(v.getViewRootContainer());
		} else {
			Stage.behind.push(v);
			stage.getCharactersBackContainer().addChild(v.getViewRootContainer());
		}
	},
	addChild: function(c) { return c; },
	// a place among them by index: the first is behind the fighters, any other in front of them
	// (clock town's balloon puts itself at the back)
	setChildIndex: function(c, n) {
		var x = Stage.engineOf(c);
		if (x == null) return;
		if (AS3.int(n) <= 0) stage.getCharactersBackContainer().addChild(x.getViewRootContainer());
		else stage.getCharactersFrontContainer().addChild(x.getViewRootContainer());
	},
	addChildAt: function(c, n) { Stage.layer.setChildIndex(c, n); return c; },
	// an effect taken off the stage is gone
	removeChild: function(c) { if (c != null && c.__vfx != null) Effects.remove(c); return c; },
	contains: function(c) { return Stage.engineOf(c) != null; }
};

// an object drawn in front of the fighters
Stage.toFront = function(me, ?piece) {
	var fm = me == null ? null : me.__fm;
	if (fm == null || fm.getViewRootContainer == null) return;
	stage.getForegroundStructuresContainer().addChild(fm.getViewRootContainer());
};

// one object put behind (or in front of) another: two game objects by the engine's own order, an
// effect by the side of the fighters it is drawn on
Stage.order = function(me, o, front) {
	var x = me.__fm;
	var y = Stage.engineOf(o);
	if (x == null || y == null || x == y) return;
	if (Stage.isEffect(y)) {
		var layer = front ? stage.getCharactersBackContainer() : stage.getCharactersFrontContainer();
		layer.addChild(y.getViewRootContainer());
		return;
	}
	if (front) x.bringInFront(y); else x.sendBehind(y);
};

// ---- the stage's surfaces --------------------------------------------------------------------

// the nearest surface a segment crosses (source units in), as a platform, or null. the source's
// options: terrains/terrain (solid ground), platforms (drop-through floors), ignoreList
Stage.between = function(ax, ay, bx, by, ?o) {
	var s = PeptideRuntime.data.units.size;
	var opt = function(k, d) { var v = o == null ? null : AS3.at(o, k); return v == null ? d : AS3.truthy(v); };
	var terrains = opt("terrains", opt("terrain", true));
	var platforms = opt("platforms", true);
	var ignore = o == null ? null : AS3.at(o, "ignoreList");
	var p1 = EnginePoint.create(AS3.num(ax) * s, AS3.num(ay) * s);
	var p2 = EnginePoint.create(AS3.num(bx) * s, AS3.num(by) * s);
	var best = null;
	var bestD = 0.0;
	for (k in [StructureType.FLOOR, StructureType.CEILING, StructureType.LEFT_WALL, StructureType.RIGHT_WALL]) {
		var out = EnginePoint.create(p2.x, p2.y);
		var t = Stage.tester(PeptideRuntime.fm);
		var hits = t == null ? null : t.hitTestStructuresWithLineSegment(p1, p2, out, { structureType: k });
		if (hits == null) continue;
		for (st in hits) {
			var drop = st.getStructureStat("dropThrough") == true;
			if (drop ? !platforms : !terrains) continue;
			if (ignore != null && Platforms.listed(ignore, st)) continue;
			var d = (out.x - p1.x) * (out.x - p1.x) + (out.y - p1.y) * (out.y - p1.y);
			if (best == null || d < bestD) { best = st; bestD = d; }
		}
	}
	return Platforms.view(best);
};

// the ledges of the stage's floors, as the source reads them (a clip with x and y)
Stage.ledges = function() {
	var s = PeptideRuntime.data.units.size;
	var out = [];
	for (st in match.getStructures()) {
		if (st.getAllPoints == null) continue;
		var pts = st.getAllPoints();
		if (pts == null || pts.length < 2) continue;
		if (st.getStructureStat("leftLedge") == true) out.push(Stage.ledgeAt(pts[0].x / s, pts[0].y / s));
		if (st.getStructureStat("rightLedge") == true) { var p = pts[pts.length - 1]; out.push(Stage.ledgeAt(p.x / s, p.y / s)); }
	}
	return out;
};
Stage.ledgeAt = function(x, y) { var c = MovieClip.create(); c.x = x; c.y = y; return c; };

// one view per surface, so two answers for the same one compare equal
var Platforms = {};
Platforms.made = [];
Platforms.view = function(st) {
	if (st == null) return null;
	// one of the faces of the body the running object gives itself is that object (a fighter
	// standing on a platform object stands on the object)
	var o = PeptideRuntime.object;
	if (o != null && o.__faces != null) for (f in o.__faces) if (f.st == st) return o;
	for (v in Platforms.made) if (v.__fm == st) return v;
	var me = {};
	PeptidePlatform.build(me);
	me.__fm = st;
	Platforms.made.push(me);
	return me;
};
Platforms.listed = function(list, st) { for (p in list) if (p != null && p.__fm == st) return true; return false; };

// class PeptideCollisionBoundary extends PeptideBaseAPIObject
var PeptideCollisionBoundary = { name: "PeptideCollisionBoundary" };
PeptideCollisionBoundary.build = function(me) {
	PeptideBaseAPIObject.build(me);
	me.__is.push("PeptideCollisionBoundary");
	me.__type = "SSF2CollisionBoundary";
	me.__surface = true;
	// what content keeps on it for itself
	me.metadata = {};
};
// a point inside a surface: this engine's ground is its top edge, solid below it, so a point is in
// it when a line straight up from the point crosses that edge (a thin platform has no inside to be
// in, and is left out). the source's drawn-shape test has nothing here to test against
PeptideCollisionBoundary.hitTestPoint = function(me, x, y, ?shape) {
	var fm = PeptideRuntime.fm;
	if (fm == null || fm.isDisposed()) return false;
	var s = PeptideRuntime.data.units.size;
	var px = AS3.num(x) * s;
	var py = AS3.num(y) * s;
	// a region the stage's tree placed is the rectangle it drew
	if (me.__rect != null) { var r = me.__rect; return px >= r.x && px <= r.x + r.w && py >= r.y && py <= r.y + r.h; }
	fm = Stage.tester(fm);
	if (fm == null) return false;
	var out = EnginePoint.create(px, py - 2000);
	var hits = fm.hitTestStructuresWithLineSegment(EnginePoint.create(px, py), out, out, { structureType: StructureType.FLOOR });
	if (hits == null) return false;
	for (h in hits) {
		if (me.__fm != null && h != me.__fm) continue;
		if (h.getStructureStat("dropThrough") == true) continue;
		return true;
	}
	return false;
};
PeptideCollisionBoundary.hitTestRect = function(me, r, ?shape) { PeptideRuntime.gap("hitTestRect: a rectangle against a surface's shape"); return false; };
PeptideCollisionBoundary.init = function(me, ?api) {};

// class PeptidePlatform extends PeptideCollisionBoundary
var PeptidePlatform = { name: "PeptidePlatform" };
PeptidePlatform.build = function(me) {
	PeptideCollisionBoundary.build(me);
	me.__is.push("PeptidePlatform");
	me.__fallthrough = false;
	me.__type = "SSF2Platform";
	// `fallthrough` as a property: written, the surface is switched as `setFallthrough` switches it
	me.get_fallthrough = function() { return me.__fallthrough; };
	me.set_fallthrough = function(v) { PeptidePlatform.setFallthrough(me, v); return v; };
};
// a path it moves along, a leg at a time: each leg moves it by its speeds (source pixels a frame)
// for its time, then holds it for its wait, and the last leads back to the first (yoshi's story's
// cloud goes right, down, left and up again)
PeptidePlatform.addMovement = function(me, m) {
	if (me.__moves == null) { me.__moves = []; me.__moveAt = 0; me.__moveT = 0.0; }
	me.__moves.push(m);
};
// a surface's own stats: the engine's structure, or every face of a body (stage boot sets the latter)
PeptidePlatform.structureStats = function(me, o) { if (me != null && me.__fm != null) me.__fm.updateStructureStats(o); };
PeptidePlatform.structureStat = function(me, k) { return me == null || me.__fm == null ? null : me.__fm.getStructureStat(k); };
PeptidePlatform.setFallthrough = function(me, v) { me.__fallthrough = AS3.truthy(v); me.__fm.updateStructureStats({ disabled: me.__fallthrough }); };
PeptidePlatform.addIgnoreObject = function(me, o) { var e = Stage.engineOf(o); if (e != null) me.__fm.addToBlacklist(e); };
PeptidePlatform.removeIgnoreObject = function(me, o) { var e = Stage.engineOf(o); if (e != null) me.__fm.removeFromBlacklist(e); };
// where it was the first time its start was asked for (where it was put, before its own code moved
// it), in the source's space
PeptidePlatform.getStartPosition = function(me) {
	if (me.__start == null) {
		var fm = me.__fm;
		var s = PeptideRuntime.data.units.size;
		var o = PeptideRuntime.data.origin;
		var ox = o == null ? 0 : AS3.num(o.x);
		var oy = o == null ? 0 : AS3.num(o.y);
		me.__start = fm == null ? Point.create(0, 0) : Point.create(fm.getX() / s + ox, fm.getY() / s + oy);
	}
	return me.__start.clone();
};
PeptidePlatform.init = function(me, ?api) {};

// class PeptideStageQuality
// the quality tiers a stage hides its costlier pieces by: this engine runs with every setting on,
// so a stage asking to be set up for the current tier has nothing to hide
var PeptideStageQuality = { name: "PeptideStageQuality" };
PeptideStageQuality.build = function(me) {
	if (me.__is == null) me.__is = [];
	me.__is.push("PeptideStageQuality");
};
PeptideStageQuality.init = function(?a, ?b, ?c, ?d, ?e, ?f) {};
