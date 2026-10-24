// A stage's objects, as the source's engine runs them: each class a stage package declares with
// code of its own (an enemy, a platform the tree places) runs on a custom game object of its own,
// and here is that engine's part around it. Its body (the floor and walls it stands for), the
// stats it declares that this engine spells another way (surviving the death bounds, taking no
// launch), its hits coming round again at the attack's refresh rate, its own path through a hit
// (no hit freeze), the clip it hides showing nothing, and what it gives back when it goes.
//
// What the object holds about itself comes from its own declarations as it runs (`ownStats`,
// `getAttackStats`); what the conversion measured outside them (a placed body's outline) is
// `DATA.object.body`.

// ---- one object made by another's code ----------------------------------------------------------
//
// The stage's code (or another object's) makes an object by its class, and goes on to use what it
// made at once: places it, calls its methods, sets its fields. The engine makes the object's
// script start either as the object is made or a frame later. Until it has started, the maker
// holds a stand-in for it (`shell`), with every method and stored field the class declares
// (`DATA.faces`, by class): what is asked of it then waits, and the object takes it up in its own
// script as it starts. Once it has started, a call is the object's own method, run in its own
// script (`Bodies.direct`), or handed to its next update (`exports.__asks`) where an engine runs
// another script's methods without that script's helpers.

var StageObjects = {};
StageObjects.installed = false;

// the members of the source's api that a stage and its objects answer differently from a fighter:
// put on the api as the script starts (a fighter's script carries none of this)
StageObjects.install = function() {
	if (StageObjects.installed == true) return;
	StageObjects.installed = true;
	var o = PeptideRuntime.data.origin;
	StageAPI.install();
	PeptideAPI.makeEnemy = function(c) { return StageObjects.make(c); };
	// a platform of a class: the object its class converted to, as an enemy is
	PeptideAPI.spawnPlatform = function(c, ?rest) { return StageObjects.make(c); };
	// the stage every object asks for: the stage's own object, once its script runs
	var stageOf = Stage.get;
	Stage.get = function() {
		var ex = stage == null ? null : stage.exports;
		if (ex != null && ex.__source != null) return ex.__source;
		return stageOf();
	};
	// places on the stage are the source's: its origin is where this engine's is not
	var rect = Stage.rect;
	Stage.rect = function(a) {
		var r = rect(a);
		if (r == null) return null;
		r.x = r.x + o.x;
		r.y = r.y + o.y;
		r.left = r.x;
		r.top = r.y;
		r.right = r.x + r.width;
		r.bottom = r.y + r.height;
		return r;
	};
	var between = Stage.between;
	PeptideAPI.getPlatformBetweenPoints = function(a, b, ?opts) { return (a == null || b == null) ? null : between(AS3.num(a.x) - o.x, AS3.num(a.y) - o.y, AS3.num(b.x) - o.x, AS3.num(b.y) - o.y, opts); };
	PeptideAPI.hitTestGroundBetweenPoints = PeptideAPI.getPlatformBetweenPoints;
	PeptideAPI.hitTestGround = function(x, y, ?opts) { return between(AS3.num(x) - o.x, AS3.num(y) - o.y - 1, AS3.num(x) - o.x, AS3.num(y) - o.y + 1, opts); };
	// a body an object gives itself: its floor, and its walls and underside when it is solid
	PeptideGameObject.makeOwnFloor = function(me, x, y, w, h, ?terrain) { return Bodies.ownFloor(me, x, y, w, h, terrain); };
	PeptideGameObject.dropOwnFloor = function(me) { Bodies.dropFloor(me); };
	// a surface that is a body's faces is all of them at once
	var fallthrough = PeptidePlatform.setFallthrough;
	PeptidePlatform.setFallthrough = function(me, v) {
		var fs = Bodies.facesOf(me);
		if (fs == null) { fallthrough(me, v); return; }
		me.__fallthrough = AS3.truthy(v);
		me.__fallSet = true;
		for (f in fs) f.st.updateStructureStats({ disabled: me.__fallthrough });
	};
	var stats = PeptidePlatform.structureStats;
	PeptidePlatform.structureStats = function(me, o) {
		var fs = Bodies.facesOf(me);
		if (fs == null) { stats(me, o); return; }
		for (f in fs) f.st.updateStructureStats(o);
	};
	var stat = PeptidePlatform.structureStat;
	PeptidePlatform.structureStat = function(me, k) {
		var fs = Bodies.facesOf(me);
		return fs == null ? stat(me, k) : (fs.length == 0 ? null : fs[0].st.getStructureStat(k));
	};
	var ignore = PeptidePlatform.addIgnoreObject;
	PeptidePlatform.addIgnoreObject = function(me, x) {
		var fs = Bodies.facesOf(me);
		var e = Stage.engineOf(x);
		if (fs == null) { ignore(me, x); return; }
		if (e != null) for (f in fs) f.st.addToBlacklist(e);
	};
	var unignore = PeptidePlatform.removeIgnoreObject;
	PeptidePlatform.removeIgnoreObject = function(me, x) {
		var fs = Bodies.facesOf(me);
		var e = Stage.engineOf(x);
		if (fs == null) { unignore(me, x); return; }
		if (e != null) for (f in fs) f.st.removeFromBlacklist(e);
	};
	// an enemy's stats, as this engine keeps those of an object of its own
	PeptideEnemy.updateEnemyStats = function(me, s) { Bodies.stats(me, s); };
	// the attack an object is in is the one it started, whether or not its clip has a label for it
	var attack = PeptideGameObject.startAttack;
	PeptideGameObject.startAttack = function(me, n, ?f, ?special) { me.__attack = AS3.str(n); return attack(me, n, f, special); };
};

// the ledges of the stage's floors, in the stage's space
StageObjects.ledges = function() {
	var o = PeptideRuntime.data.origin;
	var out = Stage.ledges();
	for (l in out) { l.x = l.x + o.x; l.y = l.y + o.y; }
	return out;
};

// the class an object is made by, by its name without its package
StageObjects.short = function(n) {
	var s = AS3.str(n);
	var i = s.lastIndexOf(".");
	return i >= 0 ? s.substr(i + 1) : s;
};

// an object made by its class: the content the package converted the class to, made on the stage
StageObjects.make = function(c) {
	var n = StageObjects.short(Std.isOfType(c, String) ? c : c.name);
	var id = AS3.at(PeptideRuntime.data.pkg.objects, n);
	if (id == null) { PeptideRuntime.gap("makeEnemy: " + n + " (a class this package does not convert)"); return null; }
	var content = PeptideRuntime.fm.getResource().getContent(id);
	if (content == null) { PeptideRuntime.report("no object " + id); return null; }
	var h = match.createCustomGameObject(content, null);
	if (h == null) return null;
	var v = StageObjects.view(h, n);
	// in the stage's layer of objects, as the source puts what it makes: a clip that only holds
	// their order (the engine draws each object itself)
	if (v != null && v.__mc != null && v.__mc.parent == null) StageObjects.layer().addChild(v.__mc);
	return v;
};
StageObjects.layer = function() {
	if (StageObjects.held == null) StageObjects.held = MovieClip.create();
	return StageObjects.held;
};

// the object for one of this engine's: the one its script runs once it has started (when calls
// go straight to it), else the stand-in its maker holds
StageObjects.view = function(h, n) {
	if (h == null) return null;
	var ex = h.exports;
	// counted on the stage, for the harness: how many objects were running their own script by the
	// time their maker had them back, and how many started later
	var home = StageObjects.home();
	if (home != null) {
		if (ex != null && ex.__source != null) home.__liveAtMake = (home.__liveAtMake == null ? 0 : home.__liveAtMake) + 1;
		else home.__startedLater = (home.__startedLater == null ? 0 : home.__startedLater) + 1;
	}
	if (Bodies.direct == true && ex != null && ex.__source != null) return ex.__source;
	for (v in Objects.made) if (v.__fm == h) return v;
	var p = { __fm: h, __early: [], __started: false, __mc: null };
	var faces = PeptideRuntime.data.faces == null ? null : PeptideRuntime.data.faces.get(n);
	if (faces != null) faces(p, StageObjects.forward, StageObjects.cell);
	Objects.made.push(p);
	var all = StageObjects.shells();
	if (all != null) all.push(p);
	return p;
};

// what the stage keeps for its objects: its exports, whichever script asks (the stage's own script
// says which they are as it starts; every other script reaches the stage as `stage`)
StageObjects.stageExports = null;
StageObjects.home = function() {
	if (StageObjects.stageExports != null) return StageObjects.stageExports;
	return stage == null ? null : stage.exports;
};

// the stand-ins not yet taken up, kept on the stage, where every object's script can find its own
StageObjects.shells = function() {
	var ex = StageObjects.home();
	if (ex == null) return null;
	if (ex.__shells == null) ex.__shells = [];
	return ex.__shells;
};

// the stand-in made for this engine object, taken off the list
StageObjects.claim = function(fm) {
	var all = StageObjects.shells();
	if (all == null) return null;
	var k = 0;
	while (k < all.length) {
		if (all[k].__fm == fm) { var p = all[k]; all.splice(k, 1); return p; }
		k++;
	}
	return null;
};

// a method of the stand-in: the call goes where the object is
StageObjects.forward = function(p, i) {
	return function(?a0, ?a1, ?a2, ?a3, ?a4, ?a5, ?a6, ?a7) { return StageObjects.send(p, i, [a0, a1, a2, a3, a4, a5, a6, a7]); };
};
// a stored field of the stand-in: written through to the object, read back as written until the
// object answers for itself
StageObjects.cell = function(p, j) {
	var c = { v: null };
	c.get = function() {
		if (p.__started == true) return StageObjects.send(p, j + 1, []);
		return c.v;
	};
	c.set = function(v) { c.v = v; StageObjects.send(p, j, [v]); return v; };
	return c;
};
// a member asked of the stand-in (`i` names it in the class's table): kept for the object until
// its script has started, then the object's own
StageObjects.send = function(p, i, a) {
	var h = p.__fm;
	if (h == null || h.isDisposed()) return null;
	var ex = h.exports;
	if (p.__started != true || ex == null) { p.__early.push({ i: i, a: a }); return null; }
	if (Bodies.direct == true && ex.__call != null) return ex.__call(i, a);
	if (ex.__asks != null) ex.__asks.push({ i: i, a: a });
	return null;
};

// ---- the object's kind ---------------------------------------------------------------------------

var Bodies = {};
// a call from another script runs at once, in the object's own code: what the engine does for a
// method one script publishes and another calls. false hands each call to the object's next
// update instead, for an engine that runs such a method without its own script's helpers
Bodies.direct = true;
// the engine class an object is when the class its package declares is no game object (a
// platform the tree places): an enemy, with the class on top
Bodies.api = function() { return PeptideEnemy; };
Bodies.entering = function(me, anim, ?again) {};
// nothing in this engine moves an object on from an animation: its clip does
Bodies.playsOut = false;
// where it comes in the source engine's pass over its objects: after the fighters
Bodies.order = 1;
Bodies.waits = function(me) { return false; };
Bodies.beforeTick = function(me) {};
// a hit never freezes it: the source's objects take a hit down their own path and run on
Bodies.frozen = function(me) { return false; };

// the script starts: the object (the stand-in its maker holds, when one was made for it), its
// clip, its constructor and its initialize, as the runtime starts any object, then what was asked
// of it before it had started
Bodies.start = function(fm, data) {
	PeptideRuntime.ended = false;
	PeptideRuntime.fm = fm;
	PeptideRuntime.data = data;
	PeptideRuntime.kind = Bodies;
	PeptideRuntime.statics(data);
	var o = data.object;
	StageObjects.install();
	var shell = StageObjects.claim(fm);
	var me = (shell != null && Bodies.direct == true) ? shell : {};
	me.__is = null;
	Bodies.api().build(me);
	o.cls.build(me);
	me.__fm = fm;
	fm.exports.__source = me;
	fm.exports.__linkageId = o.linkage;
	fm.exports.__declared = o.stats;
	PeptideRuntime.object = me;
	Rollback.decide();
	Bodies.begin(me);
	PeptideRuntime.making = true;
	o.cls.init(me, null);
	var mc = PeptideRuntime.makeClip(o.clip);
	mc.__object = me;
	me.__mc = mc;
	// in a display list, as the source's stage puts an object's clip in its own: its code orders
	// itself there (warioware's tree goes to the back). the engine draws the object, so the order
	// is the clip's to keep
	if (mc.parent == null) Bodies.layer().addChild(mc);
	PeptideRuntime.root = mc;
	PeptideRuntime.run(me.initialize);
	PeptideRuntime.making = false;
	PeptideRuntime.shown = null;
	if (shell != null) {
		shell.__started = true;
		var early = shell.__early;
		shell.__early = [];
		for (q in early) Bodies.ask(me, q);
	}
	// the engine goes to the animation that shows where its clip is now (its own code may have sent
	// it on as it was made)
	PeptideRuntime.dirty = true;
	PeptideRuntime.settle();
};

Bodies.container = null;
Bodies.layer = function() {
	if (Bodies.container == null) { Bodies.container = MovieClip.create(); Bodies.container.name = "objects"; }
	return Bodies.container;
};

// the clip an object's animations show: the stance its clip places, else its clip
Bodies.stance = function(o) {
	if (o == null) return null;
	var s = PeptideRuntime.stanceOf(o.__mc);
	return s != null ? s : o.__mc;
};

// a member asked by its place in the class's table, run in this script
Bodies.ask = function(me, q) {
	return PeptideRuntime.run(function() { return PeptideRuntime.data.object.invoke(me, q.i, q.a); });
};
// a member another script asks, run here
Bodies.call = function(i, a) {
	var me = PeptideRuntime.object;
	if (me == null || PeptideRuntime.ended == true) return null;
	return Bodies.ask(me, { i: i, a: a });
};

// as the object is made, before its constructor: what it declares about itself, its body, and
// how others reach it
Bodies.begin = function(me) {
	var fm = PeptideRuntime.fm;
	// one the tree placed is where it was placed as it is made, whatever its maker does after
	var at = PeptideRuntime.data.object.at;
	if (at != null) { fm.setX(at.x); fm.setY(at.y); }
	// a platform's class on a game object: it is no surface of the engine's
	me.__surface = null;
	me.__bodyFaces = [];
	me.__selfPlatforms = [];
	me.__rehit = 0;
	me.__rehitLabel = null;
	me.__rehitEvery = 0;
	me.__shown = true;
	me.__clipRead = false;
	fm.exports.__call = function(i, a) { return Bodies.call(i, a); };
	fm.exports.__asks = [];
	Bodies.declared(me);
	var b = PeptideRuntime.data.object.body;
	if (b != null) Bodies.placedBody(me, b);
	// hurt: the source's engine tells the object's own `onHurt` (the local `life` needs none of the
	// script's names, so a hit after the script ended is dropped first)
	var life = PeptideRuntime.life;
	fm.addEventListener(GameObjectEvent.HIT_RECEIVED, function(?e) {
		if (life.on != true || fm.isDisposed() || me.onHurt == null) return;
		// one its class already listens with hears it there
		if (Events.handled(me, PeptideEvent.ENEMY_HURT, me.onHurt).length > 0) return;
		// the source's event, its record under `data` as a listener gets it
		PeptideRuntime.run(me.onHurt, { type: "enemyHurt", target: me, data: Events.record(me, e) });
	}, { persistent: true });
};

// what the class states about itself in `ownStats`, on the engine's object
Bodies.declared = function(me) {
	var s = me.ownStats != null ? PeptideRuntime.run(me.ownStats) : PeptideRuntime.fm.exports.__declared;
	if (s == null) return;
	var want = {};
	var any = false;
	for (k in ["gravity", "max_ySpeed", "surviveDeathBounds", "canReceiveKnockback"]) {
		var v = AS3.at(s, k);
		if (v != null) { AS3.putKey(want, k, v); any = true; }
	}
	if (any) Bodies.stats(me, want);
};

// stats the object holds, and the ones this engine keeps for it in its own terms
Bodies.stats = function(me, s) {
	var fm = me.__fm;
	if (me.__stats == null) me.__stats = new haxe.ds.StringMap();
	var u = PeptideRuntime.data.units;
	for (k in AS3.keysOf(s)) {
		var v = AS3.at(s, k);
		me.__stats.set(k, v);
		if (fm == null || fm.isDisposed()) continue;
		if (k == "gravity") fm.updateGameObjectStats({ gravity: AS3.num(v) * u.accel });
		if (k == "max_ySpeed" && AS3.num(v) > 0) fm.updateGameObjectStats({ terminalVelocity: AS3.num(v) * u.speed });
		if (k == "surviveDeathBounds") fm.updateGameObjectStats({ deathBoundsDestroy: !AS3.truthy(v) });
		if (k == "canReceiveKnockback" && !AS3.truthy(v)) fm.applyGlobalBodyStatus(BodyStatus.LAUNCH_ARMOR, -1);
	}
};

// every source frame, before its own code: no hit freezes it, and what another script handed it
// runs now, in its own code
Bodies.frame = function(me) {
	var fm = me.__fm;
	if (fm.isDisposed()) return;
	if (fm.getHitstop() > 0) fm.startHitstop(0);
	var asks = fm.exports.__asks;
	if (asks != null && asks.length > 0) {
		fm.exports.__asks = [];
		for (q in asks) Bodies.ask(me, q);
	}
};

// every engine frame: its body goes where it is, a hidden clip hides it, and a lingering hit
// comes round again
Bodies.engineFrame = function(me) {
	var fm = me.__fm;
	if (fm.isDisposed()) return;
	if (me.__moves != null && me.__moves.length > 0) Bodies.move(me);
	var x = fm.getX();
	var y = fm.getY();
	for (f in me.__bodyFaces) { f.st.setX(x); f.st.setY(y); }
	for (f in me.__selfPlatforms) { f.st.setX(x); f.st.setY(y); }
	var mc = PeptideRuntime.root;
	if (mc != null && (mc.visible == true) != me.__shown) { me.__shown = mc.visible == true; fm.setVisible(me.__shown); }
	if (me.__clipRead != true && mc != null) Bodies.clipSays(me, mc);
	Bodies.rehit(me);
};

// a leg of its path, half a source frame of it (its time and wait are source frames)
Bodies.move = function(me) {
	var fm = me.__fm;
	var m = me.__moves[me.__moveAt];
	var time = AS3.num(AS3.at(m, "moveTime"));
	var wait = AS3.num(AS3.at(m, "waitTime"));
	if (me.__moveT < time) {
		var s = PeptideRuntime.data.units.size * 0.5;
		var vx = AS3.at(m, "xSpeed");
		var vy = AS3.at(m, "ySpeed");
		if (vx != null) fm.setX(fm.getX() + AS3.num(vx) * s);
		if (vy != null) fm.setY(fm.getY() + AS3.num(vy) * s);
	}
	me.__moveT += 0.5;
	if (me.__moveT >= time + wait) { me.__moveT = 0.0; me.__moveAt = (me.__moveAt + 1) % me.__moves.length; }
};

// what its clip's own first frame declares about the surface it is, once that frame has run: a
// platform that starts let through (`fallthrough`), unless its class has already said
Bodies.clipSays = function(me, mc) {
	me.__clipRead = true;
	if (me.__faces == null || me.__fallSet == true) return;
	var v = mc.__c_fallthrough != null ? mc.__c_fallthrough.get() : mc.fallthrough;
	if (AS3.truthy(v)) PeptidePlatform.setFallthrough(me, true);
};

// the attack it is in hits a fighter again every `refreshRate` of the source's frames, as long as
// the fighter stays in it (this engine hits each one once an attack otherwise)
Bodies.rehit = function(me) {
	var root = PeptideRuntime.root;
	var label = me.__attack != null ? me.__attack : (root == null ? null : root.currentLabel);
	if (label != me.__rehitLabel) {
		me.__rehitLabel = label;
		me.__rehit = 0;
		me.__rehitEvery = 0;
		var a = me.getAttackStats != null && label != null ? AS3.at(PeptideRuntime.run(me.getAttackStats), label) : null;
		var r = a == null ? null : AS3.at(a, "refreshRate");
		if (r != null && AS3.num(r) > 0) me.__rehitEvery = Std.int(AS3.num(r) * 2);
	}
	if (me.__rehitEvery <= 0) return;
	me.__rehit++;
	if (me.__rehit >= me.__rehitEvery) { me.__rehit = 0; me.__fm.reactivateHitboxes(); }
};

// ---- bodies: the surfaces an object stands for ---------------------------------------------------

// the faces of a body, made where nobody can reach them and moved onto the object each frame. the
// engine's structures cannot be taken away, so a face an object has finished with waits on the
// stage for the next one of the same shape
Bodies.spare = function() {
	var ex = StageObjects.home();
	if (ex == null) return [];
	if (ex.__spareFaces == null) ex.__spareFaces = [];
	return ex.__spareFaces;
};
Bodies.face = function(ty, pts, ledgeL, ledgeR, drop) {
	var fm = PeptideRuntime.fm;
	var key = ty + ":" + pts.join(",") + ":" + ledgeL + ledgeR + drop;
	var pool = Bodies.spare();
	var k = 0;
	var st = null;
	while (k < pool.length) {
		if (pool[k].k == key) { st = pool[k].st; pool.splice(k, 1); break; }
		k++;
	}
	if (st == null) {
		st = match.createLineSegmentStructure(pts, new StructureStats({ startX: -2000, startY: -3000, structureType: ty, leftLedge: ledgeL, rightLedge: ledgeR, dropThrough: drop }));
		if (st == null) return null;
	}
	st.updateStructureStats({ disabled: false });
	// a body never stands on its own faces
	st.addToBlacklist(fm);
	st.setX(fm.getX());
	st.setY(fm.getY());
	return { st: st, k: key };
};
Bodies.release = function(faces) {
	var fm = PeptideRuntime.fm;
	var pool = Bodies.spare();
	for (f in faces) {
		f.st.updateStructureStats({ disabled: true });
		f.st.setX(-2000);
		f.st.setY(-3000);
		if (fm != null) f.st.removeFromBlacklist(fm);
		pool.push(f);
	}
};

// the body the tree placed it as: the outline its collision clip draws, else the box it declares
Bodies.placedBody = function(me, b) {
	var runs = b.faces;
	if (runs == null || runs.length == 0) {
		var l = b.box[0]; var t = b.box[1]; var r = b.box[2]; var bt = b.box[3];
		runs = [[StructureType.FLOOR, [l, t, r, t]], [StructureType.RIGHT_WALL, [r, t, r, bt]], [StructureType.CEILING, [r, bt, l, bt]], [StructureType.LEFT_WALL, [l, bt, l, t]]];
	}
	for (run in runs) {
		var floor = run[0] == StructureType.FLOOR;
		var f = Bodies.face(run[0], run[1], floor && b.ledges[0] == true, floor && b.ledges[1] == true, false);
		if (f != null) me.__bodyFaces.push(f);
	}
	me.__faces = me.__bodyFaces;
	// a body the tree placed is part of the stage: the death bounds never take it, wherever its
	// code parks it (a platform that waits above the top of the stage before it comes down)
	PeptideRuntime.fm.updateGameObjectStats({ deathBoundsDestroy: false });
};

// the body an object gives itself (source units, its own space): a floor fighters land on from
// above and drop through, or, asked for as terrain, a solid box with ledges. one at a time
Bodies.ownFloor = function(me, x, y, w, h, terrain) {
	Bodies.dropFloor(me);
	var s = PeptideRuntime.data.units.size;
	var x0 = Math.min(AS3.num(x), AS3.num(x) + AS3.num(w)) * s;
	var x1 = Math.max(AS3.num(x), AS3.num(x) + AS3.num(w)) * s;
	var y0 = Math.min(AS3.num(y), AS3.num(y) + AS3.num(h)) * s;
	var y1 = Math.max(AS3.num(y), AS3.num(y) + AS3.num(h)) * s;
	var solid = AS3.truthy(terrain);
	var runs = solid
		? [[StructureType.FLOOR, [x0, y0, x1, y0]], [StructureType.RIGHT_WALL, [x1, y0, x1, y1]], [StructureType.CEILING, [x1, y1, x0, y1]], [StructureType.LEFT_WALL, [x0, y1, x0, y0]]]
		: [[StructureType.FLOOR, [x0, y0, x1, y0]]];
	var faces = [];
	for (run in runs) {
		var floor = run[0] == StructureType.FLOOR;
		var f = Bodies.face(run[0], run[1], floor && solid, floor && solid, floor && !solid);
		if (f != null) faces.push(f);
	}
	if (faces.length == 0) return null;
	me.__selfPlatforms = faces;
	var v = {};
	PeptidePlatform.build(v);
	v.__fm = faces[0].st;
	v.__faces = faces;
	return v;
};
Bodies.dropFloor = function(me) {
	if (me.__selfPlatforms == null || me.__selfPlatforms.length == 0) return;
	var faces = me.__selfPlatforms;
	me.__selfPlatforms = [];
	Bodies.release(faces);
};

// the faces a surface the source holds stands for: a body's, or none (an engine surface)
Bodies.facesOf = function(me) {
	return me == null ? null : me.__faces;
};

// the object is gone: its faces wait for the next object, and its script ends
Bodies.teardown = function(fm) {
	var me = PeptideRuntime.object;
	if (me != null && fm.isDisposed()) {
		Bodies.end(me);
		// the stage can still hold it (a list of what it made): it keeps what it last was as plain
		// values, and no method of a script that has ended
		if (me.__frozen != true) {
			me.__frozen = true;
			var freeze = function() { PeptideRuntime.data.object.freeze(me); };
			// ended from inside its own code (a goomba's hurt handler destroys it and goes on to set
			// its own fields), that code finishes on what it has first, as the source's does
			if (PeptideRuntime.updating == true || PeptideRuntime.depth > 0) PeptideRuntime.later.push(freeze); else freeze();
		}
	}
	PeptideRuntime.teardown(fm);
};
Bodies.end = function(me) {
	if (me.__ended == true) return;
	me.__ended = true;
	Bodies.dropFloor(me);
	if (me.__bodyFaces != null && me.__bodyFaces.length > 0) {
		var faces = me.__bodyFaces;
		me.__bodyFaces = [];
		Bodies.release(faces);
	}
};
