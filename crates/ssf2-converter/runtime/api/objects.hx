// The source's game objects, on this engine's: what a fighter's (or a projectile's) own code can
// ask of the object it runs on. `me.__fm` is the engine's object. Positions and speeds cross here
// and only here: the source's pixels are `DATA.units.size` of this engine's, its speeds (per
// source frame, in its pixels) are `DATA.units.speed` of this engine's. x speeds are
// facing-relative on both engines.

// class PeptideBaseAPIObject
var PeptideBaseAPIObject = { name: "PeptideBaseAPIObject" };
PeptideBaseAPIObject.build = function(me) {
	if (me.__is == null) me.__is = [];
	me.__is.push("PeptideBaseAPIObject");
	me.__fm = null;
	me.__vars = new haxe.ds.StringMap();
	me.__listeners = [];
	// what the source calls its kind (`getType`), for scripts that compare it
	me.__type = "SSF2BaseAPIObject";
	me.metadata = {};
};
PeptideBaseAPIObject.init = function(me, ?api) {};

// class PeptideGameObject extends PeptideBaseAPIObject
var PeptideGameObject = { name: "PeptideGameObject" };
PeptideGameObject.build = function(me) {
	PeptideBaseAPIObject.build(me);
	me.__is.push("PeptideGameObject");
	me.__selfPlatforms = [];
};
// the clip its animations show goes to the frame: a fighter's `stance`, else the object's own clip
// (a projectile's)
PeptideGameObject.playFrame = function(me, label) {
	var s = PeptideRuntime.stanceOf(me.__mc);
	if (s == null) s = me.__mc;
	if (s != null && s.gotoAndPlay != null) s.gotoAndPlay(label);
};
// a state by the source's number: a fighter's own; anything gone is dead (42)
PeptideGameObject.inState = function(me, n) {
	if ((me.__fm == null || me.__fm.isDisposed())) return AS3.int(n) == 42;
	return me.__fm.getType() == EntityType.CHARACTER ? States.is(me.__fm, AS3.int(n)) : false;
};
// ---- being hit and hitting (the source's hitstun is a freeze: this engine's hitstop) ----
PeptideGameObject.applyKnockbackSpeed = function(me, speed, angle) {
	var r = AS3.num(angle) * Math.PI / 180;
	var v = AS3.num(speed) * PeptideRuntime.data.units.speed;
	me.__fm.setXKnockback(Math.cos(r) * v);
	me.__fm.setYKnockback(-Math.sin(r) * v);
};
// a hit its code deals it outright, as the source's does: the damage, and the launch the stats
// state (base and growth against its damage and weight, at the stated direction turned the way
// the one dealing it faces), with the freeze they state (naruto's down throw sends the fighter
// its clone strikes straight up this way, where the clone alone sends it at 45)
PeptideGameObject.takeDamage = function(me, stats, ?by) {
	var fm = me.__fm;
	if (fm == null || stats == null) return true;
	var dmg = AS3.num(AS3.at(stats, "damage"));
	if (!(dmg == dmg)) dmg = 0;
	var before = fm.getDamage();
	// dealt by a fighter in a move, it pays that move's freshness as the move's boxes do, on top of
	// the damage the launch is worked out from (naruto's down throw deals 1 and 4 by code: 1.05 and
	// 4.2 in the source)
	var fresh = by != null && by.__rulesBase != null && by.__fm != null && by.__fm.getType() == EntityType.CHARACTER ? Attacks.useFresh(by) : 1.0;
	if (dmg != 0) fm.addDamage(dmg * fresh);
	var power = AS3.num(AS3.at(stats, "power"));
	// a growth it does not state is the source's 100 (naruto's down throw deals 1 with nothing else
	// stated, and the source sends the fighter off at 0 degrees with the launch growth 100 gives)
	var g0 = AS3.at(stats, "kbConstant");
	var growth = g0 == null ? 100 : AS3.num(g0);
	if (!(power == power)) power = 0;
	if (!(growth == growth)) growth = 0;
	// one with no effect deals its damage and launches nothing (as a box with none: hitbox_stats.jsonc)
	var effect = AS3.at(stats, "hasEffect");
	if ((power > 0 || growth > 0) && (effect == null || AS3.truthy(effect))) {
		var weight = fm.getType() == EntityType.CHARACTER ? fm.getCharacterStat("weight") : 100;
		var kb = PeptideAPI.knockbackOf(growth, power, null, dmg, before, weight, false);
		var dir = AS3.num(AS3.at(stats, "direction"));
		if (!(dir == dir)) dir = 0;
		var left = by != null && by.__fm != null && !by.__fm.isFacingRight();
		PeptideGameObject.applyKnockbackSpeed(me, PeptideAPI.knockbackVelocityOf(kb), left ? 180 - dir : dir);
	}
	var stun = AS3.num(AS3.at(stats, "hitStun"));
	if (stun > 0 && fm.getType() == EntityType.CHARACTER) fm.forceStartHitstop(Std.int(stun * 2));
	return true;
};
// its speed with or without its knockback, in the source's units, facing-relative like x speed
PeptideGameObject.netXSpeed = function(me, ?noNormal, ?noKnockback) {
	var f = me.__fm;
	var v = AS3.truthy(noNormal) ? (AS3.truthy(noKnockback) ? 0 : f.getXKnockback())
		: (AS3.truthy(noKnockback) ? f.getXVelocity() : f.getNetXVelocity());
	return f.flipX(v) / PeptideRuntime.data.units.speed;
};
PeptideGameObject.netYSpeed = function(me, ?noNormal, ?noKnockback) {
	var f = me.__fm;
	var v = AS3.truthy(noNormal) ? (AS3.truthy(noKnockback) ? 0 : f.getYKnockback())
		: (AS3.truthy(noKnockback) ? f.getYVelocity() : f.getNetYVelocity());
	return v / PeptideRuntime.data.units.speed;
};
PeptideGameObject.netSpeed = function(me, ?a, ?b) { var x = PeptideGameObject.netXSpeed(me, a, b); var y = PeptideGameObject.netYSpeed(me, a, b); return Math.sqrt(x * x + y * y); };
// ---- the camera and the stage ----
PeptideGameObject.attachToFloor = function(me) {
	var f = me.__fm;
	var out = EnginePoint.create(f.getX(), f.getY() + 4);
	var hits = f.hitTestStructuresWithLineSegment(EnginePoint.create(f.getX(), f.getY() - 4), out, out, { structureType: StructureType.FLOOR });
	if (hits == null || hits.length == 0) return;
	f.setY(out.y);
	f.attachToFloor(hits[0]);
};
// moved as far as solid ground lets it (source units); true when nothing was in the way
PeptideGameObject.moveIfClear = function(me, dx, dy) {
	var x = (me.__fm.getX() / PeptideRuntime.data.units.size);
	var y = (me.__fm.getY() / PeptideRuntime.data.units.size);
	var hit = Stage.between(x, y - 1, x + AS3.num(dx), y - 1 + AS3.num(dy), { platforms: false });
	if (hit == null) { me.__fm.setX(AS3.num((x + AS3.num(dx))) * PeptideRuntime.data.units.size); me.__fm.setY(AS3.num((y + AS3.num(dy))) * PeptideRuntime.data.units.size); return true; }
	return false;
};
// one self platform at a time: making one replaces the last. it is a platform (stood on from
// above, dropped through) unless it is asked for as terrain
// a self platform the shape of a clip: the clip's own extent, as the package measured it
PeptideGameObject.makeOwnFloorOf = function(me, mc, ?terrain) {
	var all = PeptideRuntime.data.clipBounds;
	var b = mc == null || mc.__linkage == null || all == null ? null : all.get(mc.__linkage);
	// a clip the code drew into itself: what it drew
	if (b == null && mc != null && mc.__vec != null) { var v = mc.__vec; b = { x: mc.x + v.x0, y: mc.y + v.y0, w: v.x1 - v.x0, h: v.y1 - v.y0 }; }
	if (b == null) { PeptideRuntime.gap("createSelfPlatformWithMC: a clip the package has no extent for"); return null; }
	var p = PeptideGameObject.makeOwnFloor(me, b.x, b.y, b.w, b.h, terrain);
	// the platform's clip is the one it was made from (the code hides it through the platform)
	if (p != null && p.__mc == null) p.__mc = mc;
	return p;
};
PeptideGameObject.makeOwnFloor = function(me, x, y, w, h, ?terrain) {
	PeptideGameObject.dropOwnFloor(me);
	var s = PeptideRuntime.data.units.size;
	var x0 = Math.min(AS3.num(x), AS3.num(x) + AS3.num(w)) * s;
	var x1 = Math.max(AS3.num(x), AS3.num(x) + AS3.num(w)) * s;
	var y0 = AS3.num(y) * s;
	var st = match.createLineSegmentStructure([x0, y0, x1, y0], null);
	if (st == null) return null;
	var solid = AS3.truthy(terrain);
	st.updateStructureStats({ structureType: StructureType.FLOOR, dropThrough: !solid, leftLedge: solid, rightLedge: solid });
	st.addToBlacklist(me.__fm);
	st.setX(me.__fm.getX());
	st.setY(me.__fm.getY());
	me.__selfPlatforms.push(st);
	var p = Platforms.view(st);
	// a platform made from a box has a clip with nothing in it
	if (p != null && p.__mc == null) p.__mc = MovieClip.create();
	return p;
};
PeptideGameObject.dropOwnFloor = function(me) {
	for (st in me.__selfPlatforms) { st.updateStructureStats({ disabled: true }); st.setX(-2000); st.setY(-3000); }
	me.__selfPlatforms = [];
};
PeptideGameObject.getNearestLedge = function(me) {
	var best = null;
	var bd = 0.0;
	for (l in Stage.ledges()) { var dx = l.x - (me.__fm.getX() / PeptideRuntime.data.units.size); var dy = l.y - (me.__fm.getY() / PeptideRuntime.data.units.size); var d = dx * dx + dy * dy; if (best == null || d < bd) { best = l; bd = d; } }
	return best;
};
// the nearest of a kind ("character", "player", "item", "projectile", "enemy")
PeptideGameObject.getNearest = function(me, type, ?skipTeam, ?skipOwner) {
	var fm = me.__fm;
	var list = (type == "character" || type == "player") ? match.getCharacters() : (type == "enemy") ? match.getCustomGameObjects() : match.getProjectiles();
	var best = null;
	var bd = 1.0e12;
	for (o in list) {
		if (o == fm || o.isDisposed()) continue;
		if (type == "item" && (o.exports == null || o.exports.__isItem != true)) continue;
		var dx = o.getX() - fm.getX();
		var dy = o.getY() - fm.getY();
		var d = dx * dx + dy * dy;
		if (d < bd) { bd = d; best = o; }
	}
	return Objects.view(best);
};
// with no beacons the path is the target itself
PeptideGameObject.getNearestPath = function(me, type, ?skipTeam) { var t = PeptideGameObject.getNearest(me, type, skipTeam); return t == null ? [] : [t]; };
// sent at a target, as the source's engine does it: the angle from where it is to where the
// target stands, taken the source's way (its y term is the target's height negated, minus its
// own: not the difference), the speed along that angle, and a turn to face the way it goes
PeptideGameObject.homeTowardsTarget = function(me, speed, target) {
	if (target == null) return;
	var s = PeptideRuntime.data.units.size;
	var tx = target.__fm != null ? target.__fm.getX() / s : AS3.num(target.x);
	var ty = target.__fm != null ? target.__fm.getY() / s : AS3.num(target.y);
	var fm = me.__fm;
	var a = Math.atan2(-ty - fm.getY() / s, tx - fm.getX() / s);
	var xs = AS3.num(speed) * Math.cos(a);
	var ys = -AS3.num(speed) * Math.sin(a);
	if (xs <= 0) fm.faceLeft(); else fm.faceRight();
	fm.setXSpeed(Math.abs(xs) * PeptideRuntime.data.units.speed);
	fm.setYSpeed(ys * PeptideRuntime.data.units.speed);
};
// ---- how it looks ----
PeptideGameObject.paletteData = function(me) {
	if (me.__palette != null) return me.__palette;
	return { paletteSwap: { colors: [], replacements: [] }, paletteSwapPA: { colors: [], replacements: [] } };
};
// a clip it owns takes its colours: a projectile its costume; an effect has none to take
PeptideGameObject.applyPalette = function(me, mc) {
	var e = Stage.engineOf(mc);
	if (e == null || e == me.__fm || Stage.isEffect(e)) return;
	e.setCostumeIndex(me.__fm.getCostumeIndex());
};
PeptideGameObject.getHeight = function(me) { var v = AS3.at(PeptideGameObject.ownStats(me), "height"); return v == null ? 0 : v; };
PeptideGameObject.getWidth = function(me) { var v = AS3.at(PeptideGameObject.ownStats(me), "width"); return v == null ? 0 : v; };
// the box its body takes up where it stands, in the space of the code asking (`o`: that space's
// origin): its declared width and height over its feet, else the engine's collision box
PeptideGameObject.boundsRect = function(me, ox, oy) {
	var fm = me.__fm;
	if (fm == null) return Rectangle.create();
	var size = PeptideRuntime.data.units.size;
	var x = fm.getX() / size + ox;
	var y = fm.getY() / size + oy;
	var w = AS3.num(PeptideGameObject.getWidth(me));
	var h = AS3.num(PeptideGameObject.getHeight(me));
	if (w <= 0 || h <= 0) {
		try {
			w = Math.abs(fm.getEcbRightHipX() - fm.getEcbLeftHipX()) / size;
			h = Math.abs(fm.getEcbFootY() - fm.getEcbHeadY()) / size;
		} catch (err:Dynamic) {}
	}
	return Rectangle.create(x - w / 2, y - h, w, h);
};
// how see-through it is: its engine object, and the clip its code holds
// its attack hits again: who it has hit is forgotten, and the engine's boxes come back
PeptideGameObject.reactivate = function(me) {
	me.__hitFoes = null;
	me.__fm.reactivateHitboxes();
	Attacks.carry(me, true);
};
PeptideGameObject.setAlpha = function(me, a) {
	var v = AS3.num(a);
	if (me.__mc != null) me.__mc.alpha = v;
	try { if (me.__fm != null) me.__fm.setAlpha(v); } catch (err:Dynamic) {}
};
PeptideGameObject.getAlpha = function(me) { return me.__fm != null ? me.__fm.getAlpha() : (me.__mc != null ? me.__mc.alpha : 1); };
// its declared stats, which every object's script puts on its exports
// (one this package did not convert, a stock fighter, declares nothing: an empty record)
PeptideGameObject.ownStats = function(me) {
	var d = me.__fm.exports == null ? null : me.__fm.exports.__declared;
	return d == null ? {} : d;
};
PeptideGameObject.getGameObjectStat = function(me, n) {
	var fm = me.__fm;
	if (n == "canReceiveKnockback" || n == "canReceiveHits") return fm.getType() == EntityType.CHARACTER;
	var v = AS3.at(PeptideGameObject.ownStats(me), n);
	if (v != null) return v;
	return fm.getGameObjectStat(n);
};
// a move started by the label it is under (a projectile's own, or a fighter's); a clip already
// in that move ignores it
PeptideGameObject.startAttack = function(me, n, ?f, ?special) {
	var mc = me.__mc;
	if (mc == null) return false;
	if (me.__fm.getType() != EntityType.CHARACTER && mc.currentLabel == n) return false;
	mc.gotoAndPlay(AS3.str(n));
	var st = PeptideRuntime.stanceOf(me.__mc);
	if (st != null && f != null && f != true) st.gotoAndPlay(f);
	return true;
};
PeptideGameObject.init = function(me, ?api) {};

// class PeptideCharacter extends PeptideGameObject
var PeptideCharacter = { name: "PeptideCharacter" };
PeptideCharacter.build = function(me) {
	PeptideGameObject.build(me);
	me.__is.push("PeptideCharacter");
	me.__stats = new haxe.ds.StringMap();
	me.__type = "SSF2Character";
};
// its stats: what its code set, else what it declares; a fighter no converted script runs
// answers the engine's own, for a stat it keeps
PeptideCharacter.getCharacterStat = function(me, n) {
	var ex = me.__fm.exports;
	var src = ex == null || ex.__source == null ? me : ex.__source;
	if (src.__stats != null && src.__stats.exists(n)) return src.__stats.get(n);
	if (ex != null && ex.__declared != null) {
		var v = AS3.at(ex.__declared, n);
		return v != null ? v : PeptideCharacter.statDefault(n);
	}
	var name = AS3.at(PeptideRuntime.data.stats.names, n);
	if (name == null) name = n;
	// a fighter's names are text: one that states none here has an empty one (a stage writes it
	// into its HUD lowercased: kingdom1)
	if (name == "displayName" || name == "statsName") return "";
	return PeptideRuntime.data.stats.record(name, 0) == null ? PeptideCharacter.statDefault(n) : me.__fm.getCharacterStat(name);
};
// a stat no character states reads as the source reads it (whether a foe can be launched at all)
PeptideCharacter.statDefault = function(n) {
	var d = PeptideRuntime.data.stats == null ? null : PeptideRuntime.data.stats.defaults;
	return d == null ? null : d(n);
};
PeptideCharacter.swapDepthGrabbed = function(me, ?front) {
	var g = me.__fm.getAllGrabbedFoes();
	if (g == null || g.length == 0) return;
	if (front == true) me.__fm.bringInFront(g[0]); else me.__fm.sendBehind(g[0]);
};
// the one it holds shows one of its own hurt poses, by label, when it runs the source's clips
PeptideCharacter.grabbedHurtFrame = function(me, label) {
	var g = me.__fm.getAllGrabbedFoes();
	if (g == null || g.length == 0) return;
	var o = Objects.view(g[0]);
	if (o.__mc != null) o.__mc.gotoAndStop(label);
};
// this engine's computer players press the same controls a person does, so a computer
// fighter takes the person's branch of the source's code
// ---- attacks and damage ----
// the source's rocket: a fighter is put into the attack it rockets in, at that attack's `rocket`
// frame, and carried by the speed and angle given, losing `decay` of it a frame, while that
// attack lasts. which attack that is the source keeps per fighter, by the stats name it declares
// (samus rides her bomb's lift in her air down special); a fighter it names none for is left as
// it is
PeptideCharacter.rocketAttack = function(name) {
	switch (name) {
		case "samus": return "b_down_air";
		case "ness": return "b_up_air";
		default: return null;
	}
};
PeptideCharacter.rocket = function(me, speed, angle, decay, rotate) {
	var fm = me.__fm;
	if (fm == null || fm.getType() != EntityType.CHARACTER || fm.exports == null) return;
	var attack = PeptideCharacter.rocketAttack(AS3.str(PeptideCharacter.getCharacterStat(me, "statsName")));
	if (attack == null) return;
	// not on or climbing a ledge, grabbing, hurt or launched
	for (n in [10, 13, 14, 15, 19, 20]) if (States.is(fm, n)) return;
	// in an attack, only in a down special
	var label = me.__mc == null ? null : me.__mc.currentLabel;
	if (States.is(fm, 9) && (label == null || AS3.str(label).indexOf("b_down") < 0)) return;
	if (States.is(fm, 22) || States.is(fm, 16) || States.is(fm, 18)) fm.toState(CState.STAND);
	if (AS3.truthy(rotate)) PeptideRuntime.gap("rocketFighter: the fighter turned to its rocket's angle");
	var a = AS3.num(angle) * Math.PI / 180;
	var sp = AS3.num(speed);
	var d = Math.abs(AS3.num(decay));
	// started in the fighter's own script, which runs its clip
	if (fm.exports.__startRocket != null) fm.exports.__startRocket({ attack: attack, vx: sp * Math.cos(a), vy: -sp * Math.sin(a), dx: d * Math.abs(Math.cos(a)), dy: d * Math.abs(Math.sin(a)), n: 0, anim: null });
};
PeptideCharacter.ejectOpponent = function(me) { me.__fm.releaseAllCharacters(); PeptideRuntime.gap("ejectOpponent: the launch of the one spat out"); };
// ---- grabs ----
// asked of the grabber it lets go; asked of the one held, whoever holds it lets go
PeptideCharacter.grabRelease = function(me) {
	var fm = me.__fm;
	var held = fm.getAllGrabbedFoes();
	if (held != null && held.length > 0) { fm.releaseAllCharacters(); return; }
	for (c in match.getCharacters()) { var h = c.getAllGrabbedFoes(); if (h != null && h.indexOf(fm) >= 0) c.releaseCharacter(fm); }
};
// held by the character with that id (or the fighter asking): true when it is
PeptideCharacter.grab = function(me, ?uid, ?visible, ?inv, ?lock) {
	var g = null;
	if (uid != null) for (c in match.getCharacters()) if (c.getUid() == uid) g = c;
	if (g == null) { var o = PeptideAPI.characterOf(PeptideRuntime.object); g = o == null ? null : o.__fm; }
	// only a fighter grabs here: a hold by no fighter (rainbow route's mirrors take a fighter in)
	// is the source's own state, which this engine has no way to put a fighter in
	if (g != null && g != me.__fm && !PeptideRuntime.isFighter(g)) { PeptideRuntime.gap("grab: a fighter held by no fighter (the stage's own hold)"); return false; }
	if (g == null || g == me.__fm) return false;
	g.attemptGrab(me.__fm);
	var h = g.getAllGrabbedFoes();
	return h != null && h.indexOf(me.__fm) >= 0;
};
PeptideCharacter.toGrabbed = function(me) { var g = me.__fm.getGrabbedFoe(); if (g != null) { me.__fm.setX(g.getX()); me.__fm.setY(g.getY()); } };
PeptideCharacter.toGrabbing = function(me, ?e) {
	var g = me.__fm.getAllGrabbedFoes();
	if (g == null || g.length != 1) return;
	if (!me.__fm.isOnFloor()) me.__fm.releaseAllCharacters(); else me.__fm.toState(CState.GRAB_HOLD);
};
// let go of by whoever holds it
// ---- the match around it ----
PeptideCharacter.getTeammates = function(me) {
	var out = [];
	for (c in match.getCharacters()) if (c != me.__fm && c.getTeam() == me.__fm.getTeam()) out.push(Objects.view(c));
	return out;
};
PeptideCharacter.init = function(me, ?api) {};

// The source's object for one of this engine's: the one a converted script runs (published on
// its exports as `__source`), or a view of it made once and kept
var Objects = {};
// which way an object faces, as its code sees it. a fighter faces the way this engine has it. what
// isn't a fighter faces right in the source until its own code turns it: thrown by a fighter facing
// left it flies left and still says right (link's boomerang, thrown left, slows by reading its
// speed and turning it by its facing, and here turned straight back into his hand). this engine
// turns it with its thrower, so the way its code turned it is kept apart
Objects.facingRight = function(fm) {
	if (fm.getType() == EntityType.CHARACTER || fm.exports == null) return fm.isFacingRight();
	return fm.exports.__facing == null ? true : fm.exports.__facing == true;
};
Objects.face = function(fm, right) {
	if (fm.getType() != EntityType.CHARACTER && fm.exports != null) fm.exports.__facing = right;
	if (right) fm.faceRight(); else fm.faceLeft();
};
// a fighter's slowdown a source frame, in this engine's units: the animation's own where it states
// one, the fighter's otherwise (the rules that slow a fighter in its attack pay it)
Objects.slideStep = function(fm, ?air) {
	if (fm.getType() != EntityType.CHARACTER) return 0;
	if (air == null) air = !fm.isOnFloor();
	var f = fm.getAnimationStat(air ? "aerialFriction" : "friction");
	if (f == null || f < 0) f = fm.getCharacterStat(air ? "aerialFriction" : "friction");
	return f == null ? 0 : f * 1.5;
};
Objects.slowed = function(v, s) {
	if (s == 0 || v == 0) return v;
	return v > 0 ? Math.max(v - s, 0) : Math.min(v + s, 0);
};
// what isn't a fighter reads and sets its x speed along the way it was thrown in the source, and
// says it faces right until its own code turns it; turning it changes neither (link's arrow and
// boomerang and simon's cross, thrown left, slow and turn on the source's frames; chibi-robo's
// telly vision turns to face where it steers and steers on). this engine keeps the speed along
// the way the object faces now, which is the throw's until its code turns it. one turn serves both ways
Objects.alongThrow = function(fm, v) {
	var thrown = fm.exports == null || fm.exports.__thrownRight == null ? true : fm.exports.__thrownRight == true;
	return fm.isFacingRight() == thrown ? v : -v;
};
// an x speed as the source's code reads and writes it: a fighter's as it is (a write noted, so the
// slowdown an attack starts with leaves it alone), anything else's along its throw
Objects.xSpeed = function(fm) {
	return fm.getType() == EntityType.CHARACTER ? fm.getXSpeed() : Objects.alongThrow(fm, fm.getXSpeed());
};
Objects.setXSpeed = function(fm, v) {
	if (fm.getType() != EntityType.CHARACTER) { fm.setXSpeed(Objects.alongThrow(fm, v)); return; }
	if (fm.exports != null) fm.exports.__wroteX = PeptideRuntime.frame;
	fm.setXSpeed(v);
};
// taken off: a surface stops being one; an item goes at once; anything else ends the way the
// source's engine ends a projectile, telling whoever listens for its end
Objects.destroy = function(me) {
	if (me.__surface == true) { me.__fm.updateStructureStats({ disabled: true }); return; }
	if (me.__fm == null || me.__fm.isDisposed()) return;
	if (me.__fm.exports != null && me.__fm.exports.__isItem == true) me.__fm.kill(); else Projectiles.end(me);
};
// taken off by the runtime: the object's own `destroy` where its code has one
Objects.remove = function(me) { if (me.destroy != null) me.destroy(); else Objects.destroy(me); };
// back to rest: a fighter stands; anything else plays its idle
Objects.toIdle = function(me) { if (me.__fm.getType() == EntityType.CHARACTER) me.__fm.toState(CState.STAND); else PeptideItem.attack(me, "attack_idle"); };
// thrown: a fighter throws what it holds; anything else plays its toss when it can be tossed
Objects.toToss = function(me) {
	if (me.__fm.getType() == EntityType.CHARACTER) { me.__fm.exports.__askToss = true; return; }
	if (PeptideItem.getItemStat(me, "canToss") != false) PeptideItem.attack(me, "attack_toss");
};
Objects.made = [];
Objects.view = function(fm) {
	if (fm == null) return null;
	if (fm.exports != null && fm.exports.__source != null) return fm.exports.__source;
	for (v in Objects.made) if (v.__fm == fm) return v;
	var me = {};
	if (fm.getType() == EntityType.CHARACTER) PeptideCharacter.build(me); else PeptideGameObject.build(me);
	me.__fm = fm;
	Objects.made.push(me);
	return me;
};
// the object this script already holds for it, where it holds one: a projectile it fired before
// that projectile's own script began is held as the stand-in it was given, and an event naming it
// names that same one (naruto's down throw asks whether what hurt the one it holds is the clone it
// fired); anything else as Objects.view
Objects.held = function(fm) {
	if (fm == null) return null;
	for (v in Objects.made) if (v.__fm == fm) return v;
	return Objects.view(fm);
};
Objects.views = function(list) {
	var out = [];
	if (list != null) for (x in list) out.push(Objects.view(x));
	return out;
};

// The other kinds of game object content can hold: an enemy and a target are game objects
// with an owner; what a stage does with them is the stage's.
// class PeptideEnemy extends PeptideGameObject
var PeptideEnemy = { name: "PeptideEnemy" };
PeptideEnemy.build = function(me) {
	PeptideGameObject.build(me);
	me.__is.push("PeptideEnemy");
	me.__type = "SSF2Enemy";
};
// its stats as it holds them (it runs as a projectile here: Projectiles keeps them)
PeptideEnemy.updateEnemyStats = function(me, o) {
	if (me.__stats == null) me.__stats = new haxe.ds.StringMap();
	for (k in AS3.keysOf(o)) {
		var v = AS3.at(o, k);
		me.__stats.set(k, v);
		if (k == "gravity") me.__fm.updateProjectileStats({ gravity: AS3.num(v) * PeptideRuntime.data.units.speed * 0.5 });
	}
};
PeptideEnemy.init = function(me, ?api) {};
// a fade its code steps once a frame, as it asks (a tenth of the way a source frame); fading while
// it has not got there
PeptideEnemy.fadeStep = 0.1;
PeptideEnemy.fade = function(me, to) {
	var fm = me.__fm;
	var a = fm.getAlpha();
	var d = PeptideEnemy.fadeStep;
	a = to > a ? Math.min(to, a + d) : Math.max(to, a - d);
	fm.setAlpha(a);
	me.__fading = a != to;
};
PeptideEnemy.fadeIn = function(me) { PeptideEnemy.fade(me, 1); };
PeptideEnemy.fadeOut = function(me) { PeptideEnemy.fade(me, 0); };
PeptideEnemy.isFading = function(me) { return me.__fading == true; };
PeptideEnemy.resetFade = function(me) { me.__fading = false; me.__fm.setAlpha(1); };
// class PeptideTarget extends PeptideGameObject
var PeptideTarget = { name: "PeptideTarget" };
PeptideTarget.build = function(me) {
	PeptideGameObject.build(me);
	me.__is.push("PeptideTarget");
	me.__type = "SSF2Target";
	// the game's own target: it stands until it is struck, then plays its stance's `break` and is
	// gone once that has played (its state is what a stage's code asks of it)
	me.__tstate = 0; // idle
	// it hangs where it is put: no fall, no launch, and no end at the bounds
	me.ownStats = function() { return AS3.object(["gravity", 0, "surviveDeathBounds", true, "canReceiveKnockback", false]); };
	me.onHurt = function(?e) { PeptideTarget.strike(me); };
	me.update = function() { PeptideTarget.frame(me); };
};
PeptideTarget.init = function(me, ?api) {};
PeptideTarget.state = function(me, s) {
	me.__tstate = s;
	if (me.__fm != null && me.__fm.exports != null) me.__fm.exports.__tstate = s;
};
PeptideTarget.strike = function(me) {
	if (me.__tstate != 0) return; // not idle
	PeptideTarget.state(me, 1); // broken
	var st = Bodies.stance(me);
	if (st != null) st.gotoAndPlay("break");
	Events.dispatch(me, "targetBroken");
	Events.dispatch(PeptideAPI, "targetBroken", { target: me });
};
PeptideTarget.frame = function(me) {
	if (me.__tstate != 1) return; // not broken
	var st = Bodies.stance(me);
	if (st == null || st.currentLabel == "break") return;
	PeptideTarget.state(me, 2); // dead
	Objects.destroy(me);
};
// the match's clock and its modes and menus are this engine's
// class PeptideGameTimer extends PeptideBaseAPIObject
var PeptideGameTimer = { name: "PeptideGameTimer" };
PeptideGameTimer.build = function(me) {
	PeptideBaseAPIObject.build(me);
	me.__is.push("PeptideGameTimer");
};
PeptideGameTimer.init = function(me, ?api) {};
// class PeptideCustomMatch extends PeptideBaseAPIObject
var PeptideCustomMatch = { name: "PeptideCustomMatch" };
PeptideCustomMatch.build = function(me) { PeptideBaseAPIObject.build(me); me.__is.push("PeptideCustomMatch"); };
PeptideCustomMatch.init = function(me, ?api) {};
// class PeptideCustomMenu extends PeptideBaseAPIObject
var PeptideCustomMenu = { name: "PeptideCustomMenu" };
PeptideCustomMenu.build = function(me) { PeptideBaseAPIObject.build(me); me.__is.push("PeptideCustomMenu"); };
PeptideCustomMenu.init = function(me, ?api) {};
// class PeptideCustomMode extends PeptideBaseAPIObject
var PeptideCustomMode = { name: "PeptideCustomMode" };
PeptideCustomMode.build = function(me) { PeptideBaseAPIObject.build(me); me.__is.push("PeptideCustomMode"); };
PeptideCustomMode.init = function(me, ?api) {};
