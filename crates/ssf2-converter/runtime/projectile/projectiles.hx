// Projectiles, as the source's engine runs them. A projectile's package declares its physics as
// stats (its launch, lifetime, decay, homing, the stick's pull on it, bounce, trail, how many
// may be out) and the source's engine moves it by them: no projectile's own code does. So that
// engine's part is here, reading the stats the object holds as it runs (its own code can change
// them with `updateProjectileStats`), and everything else is the projectile's own class and clip.
//
// Speeds are the source's units per source frame; this engine steps twice a source frame, so a
// per-frame change is taken half at a time on each of its frames.

var Projectiles = {};
// the engine class a projectile is, whatever class its package declares for it
Projectiles.api = function() { return PeptideProjectile; };
Projectiles.entering = function(me, anim, ?again) {};

// its clip moving on into another animation's frames plays that animation (nothing in this
// engine moves a projectile on from one)
Projectiles.playsOut = false;
Projectiles.afterSwitch = function(me) { Attacks.carry(me); };
// where it comes in the source engine's pass over its objects: after the fighters
Projectiles.order = 1;
// a projectile made in the projectiles' own pass waits a source frame before its clip starts
Projectiles.waits = function(me) {
	if (me.__late == null || me.__late <= 0) return false;
	me.__late--;
	if (me.__late > 0) return true;
	if (!me.__fm.isDisposed()) me.__fm.resume();
	return false;
};
// its clip keeps its own stops (measured: a held frame stays where it is while it flies)
// its animation clip plays on every frame, as a fighter's does, whatever a stop on it says (ness's
// pk flash is sent to `boom` with a gotoAndStop and plays its blast out to the frame that ends it)
Projectiles.beforeTick = function(me) {
	var st = PeptideRuntime.stanceOf(me.__mc);
	if (st != null) st.__playing = true;
};
Projectiles.frozen = function(me) { var fm = me.__fm; return !fm.isDisposed() && fm.getHitstop != null && fm.getHitstop() > 0; };

// what a projectile has in the source when its declaration states nothing
Projectiles.defaults = new haxe.ds.StringMap();
Projectiles.defaults.set("time_max", 60);
// not held in place unless it says so: code that asks reads false, as the source's does
// (krystal's grenade explodes only when it is not suspended, and a null was never false)
Projectiles.defaults.set("suspend", false);

// ---- firing one ------------------------------------------------------------------------------

// `fireProjectile(name, x, y, place)` from any object: the projectile its package declares by
// that name, placed off whoever fires it (x, y are an offset on top of the declared one, as
// measured on the source's record: falco's laser fired at (0, -10) with -14 declared leaves at
// -24; asked as the place itself when `place` is set and they are not both 0), launched the way
// it faces
Projectiles.fire = function(owner, name, ?x, ?y, ?place, ?stats) {
	var fm = owner.__fm;
	// fired by a stats record instead of a name: the declaration of the same clip, with the
	// record's stats
	var given = (name != null && !Std.isOfType(name, String)) ? name : null;
	var n = given == null ? AS3.str(name) : Projectiles.nameOf(AS3.at(given, "linkage_id"));
	var decl = given != null ? given : AS3.at(PeptideRuntime.data.pkg.projectiles, n);
	var id = AS3.at(PeptideRuntime.data.pkg.contents, n);
	var content = id == null ? null : fm.getResource().getContent(id);
	if (content == null) { PeptideRuntime.report("no projectile " + n); return null; }
	var u = PeptideRuntime.data.units;
	var ox = AS3.num(x == null ? 0 : x);
	var oy = AS3.num(y == null ? 0 : y);
	var dx = AS3.num(Projectiles.declared(decl, ["xoffset", "xOffset", "x_offset"]));
	var dy = AS3.num(Projectiles.declared(decl, ["yoffset", "yOffset", "y_offset"]));
	// a thrower in a throw it turns for faces the way it will turn to: the source turned it as
	// the throw began (fox's back throw fires its lasers after its victim), this engine turns it
	// once the throw is over (attack_rules.hx)
	var turned = owner.__turnAtEnd == true;
	var flipX = function(v) { return turned ? -fm.flipX(v) : fm.flipX(v); };
	var px = fm.getX() + flipX((ox + dx) * u.size);
	var py = fm.getY() + (oy + dy) * u.size;
	if (AS3.truthy(place) && (ox != 0 || oy != 0)) { px = ox * u.size; py = oy * u.size; }
	// fired into the floor its fighter stands on, it starts on that floor, as the source's does
	// (isaac's up special fires its scoop 30 below his feet: on the ground it sits on the floor)
	else if (fm.getType() == EntityType.CHARACTER && fm.isOnFloor() && py > fm.getY()) py = fm.getY();
	// the launch it declares (a speed cap stands in where no launch is stated)
	var xs = Projectiles.declared(decl, ["xspeed", "xSpeed", "x_speed"]);
	if (xs == null) xs = Projectiles.declared(decl, ["groundSpeedCap", "ground_speed_cap", "aerialSpeedCap", "aerial_speed_cap"]);
	var ys = Projectiles.declared(decl, ["yspeed", "ySpeed", "y_speed"]);
	// made by code of an object the source's engine updates as late as a projectile or later (another
	// projectile), it starts on the next pass: the source's engine updates fighters, then
	// projectiles, and one made in the projectiles' pass waits for the next to move on
	var late = (PeptideRuntime.kind != null && PeptideRuntime.kind.order >= Projectiles.order) || (fm.exports != null && fm.exports.__inPass != null && fm.exports.__inPass > 0);
	var g = Projectiles.declared(decl, ["gravity"]);
	var gmax = Projectiles.declared(decl, ["maxgravity"]);
	var launch = { x: px, y: py, xs: null, ys: null, name: n, decl: given, late: late, g: g == null ? 0 : AS3.num(g) * u.accel,
		max: gmax == null ? null : AS3.num(gmax) * u.speed };
	if ((xs != null && xs != 0) || (ys != null && ys != 0)) {
		launch.xs = flipX(AS3.num(xs) * u.speed);
		launch.ys = AS3.num(ys) * u.speed;
	}
	// whose it is: the fighter's, when another projectile makes it (the source's projectiles all
	// belong to a fighter, and this engine ends one along with the projectile that owns it)
	var own = fm;
	if (launch.late && fm.getOwner != null && fm.getOwner() != null && !fm.getOwner().isDisposed()) own = fm.getOwner();
	// the launch comes first, as the source's engine gives it before the projectile's own code
	// runs (which may change it): the projectile takes it up as it starts
	own.exports.__launch = launch;
	var p = match.createProjectile(content, own);
	if (p == null) { own.exports.__launch = null; return null; }
	if (turned) { if (p.isFacingRight()) p.faceLeft(); else p.faceRight(); }
	// the way it is thrown, which its speeds are stated along (Objects.alongThrow)
	if (p.exports != null) p.exports.__thrownRight = p.isFacingRight();
	// the stats it is fired with stand over its own for its life (krystal's grenade reads the
	// charge its toss was given, and throws as far as that charge says)
	if (stats != null && p.exports != null) p.exports.__fired = stats;
	if (own.exports.__launch != null) { own.exports.__launch = null; Projectiles.launch(p, launch); }
	// the one its maker has out now
	fm.exports.__lastProj = p;
	return Objects.view(p);
};

// the declared name of a projectile by its clip
Projectiles.nameOf = function(linkage) {
	var table = PeptideRuntime.data.pkg.projectiles;
	for (k in AS3.keysOf(table)) if (AS3.at(AS3.at(table, k), "linkage_id") == linkage) return k;
	return AS3.str(linkage);
};

Projectiles.declared = function(decl, keys) {
	if (decl == null) return null;
	for (k in keys) { var v = AS3.at(decl, k); if (v != null) return v; }
	return null;
};

Projectiles.launch = function(p, l) {
	// kept for the first floor touch it is told of (see Events.handler)
	if (p.exports != null) p.exports.__launched = l;
	p.setX(l.x);
	p.setY(l.y);
	if (l.xs != null) {
		p.setXSpeed(p.flipX(l.xs));
		p.setYSpeed(l.ys);
	}
};

// its x speed across the world: this engine keeps a projectile's along the way it faces
Projectiles.xSpeed = function(fm) { return fm.flipX(fm.getXSpeed()); };

// ---- the kind's upkeep -------------------------------------------------------------------------

// a stat as the projectile holds it now
Projectiles.stat = function(me, k) {
	// one whose script has ended (or never ran here) answers what it declared
	if (me.__stats == null) {
		var ex = me.__fm == null ? null : me.__fm.exports;
		var v = ex == null ? null : AS3.at(ex.__declared, k);
		return v != null ? v : Projectiles.defaults.get(k);
	}
	if (me.__stats.exists(k)) return me.__stats.get(k);
	return Projectiles.defaults.get(k);
};
Projectiles.num = function(me, k, d) { var v = Projectiles.stat(me, k); return v == null ? d : AS3.num(v); };

Projectiles.begin = function(me) {
	var fm = me.__fm;
	me.__stats = new haxe.ds.StringMap();
	var owner = fm.getOwner();
	// the declaration that fired it (several can share this clip), else its own
	var fired = owner != null && owner.exports != null && owner.exports.__launch != null ? owner.exports.__launch.name : null;
	var given = owner != null && owner.exports != null && owner.exports.__launch != null ? owner.exports.__launch.decl : null;
	var decl = given != null ? given : (fired == null ? null : AS3.at(PeptideRuntime.data.pkg.projectiles, fired));
	if (decl == null) decl = PeptideRuntime.data.object.stats;
	for (k in AS3.keysOf(decl)) me.__stats.set(k, AS3.at(decl, k));
	// the launch its maker handed over
	if (owner != null && owner.exports != null && owner.exports.__launch != null) {
		var l = owner.exports.__launch;
		owner.exports.__launch = null;
		Projectiles.launch(fm, l);
		// held on its first frame for one of the source's (its animation paused: set back to its
		// first frame the engine would stop)
		if (l.late == true) { me.__late = 2; fm.pause(); }
	}
	if (owner != null) fm.setCostumeIndex(owner.getCostumeIndex());
	if (AS3.truthy(Projectiles.stat(me, "canBeReversed")) || Projectiles.stat(me, "canBeReversed") == null) {
		Common.enableReflectionListener({ mode: "X", replaceOwner: true });
	}
	fm.setState(PState.ACTIVE);
	// one that states no gravity flies flat (a 0 in this engine's stats reads as unset)
	if (Projectiles.num(me, "gravity", 0) == 0) fm.updateProjectileStats({ gravity: 0 });
	Projectiles.limit(me);
	me.__age = 0;
	me.__trailT = 0;
	me.__bounces = 0;
	if (Projectiles.num(me, "bounce", 0) != 0) {
		fm.addEventListener(EntityEvent.COLLIDE_FLOOR, function(e) { if (!fm.isDisposed()) Projectiles.bounce(me); }, { persistent: true });
	}
	// a fighter its owner holds is not hit by a box of it with no effect (fox's down throw fires
	// its lasers, stated with none, into the one it holds, and only the throw's own box hits it in
	// the source; falco's, with one, hit it before the throw does), and one already hit by the
	// attack that fired it is not hit again
	fm.addEventListener(GameObjectEvent.HIT_DEALT, function(?e) { if (e != null && e.data != null && !fm.isDisposed()) Attacks.freshDealt(me, e.data.foe); }, { persistent: true });
	fm.addEventListener(GameObjectEvent.HITBOX_CONNECTED, function(?e) {
		var o = fm.getOwner();
		var hs = e.data.hitboxStats;
		if (hs == null) return;
		// what its code wrote to the box holds on the hit itself: a box that has only just come
		// out with a new animation keeps the stats it was made with (samus's bomb's blast hit for
		// its fuse's nothing)
		if (me.__atkWrites != null && hs.index != null) for (w in me.__atkWrites) {
			var n = AS3.int(w.i) - 1;
			if ((n < 0 ? 0 : n) != hs.index) continue;
			Hitboxes.onto(hs, Hitboxes.toEngine(w.stats));
		}
		var held = o == null || o.getAllGrabbedFoes == null ? null : o.getAllGrabbedFoes();
		var effect = hs.index == null || hs.index < 0 ? null : Attacks.boxStat(me, hs.index + 1, "hasEffect");
		if (effect != null && !AS3.truthy(effect) && held != null) for (h in held) if (h == e.data.foe) { hs.disabled = true; return; }
		if (me.__hitFoes == null) me.__hitFoes = [];
		if (me.__hitFoes.indexOf(e.data.foe) >= 0) { hs.disabled = true; return; }
		// a box that turns for a fighter behind it judges behind from where it was before this
		// frame's move, as the source checks its hits before it moves (a fast shot already past
		// the fighter's middle still sends it the way it flies)
		if (hs.reversibleAngle != false) {
			var before = fm.getX() - Projectiles.xSpeed(fm);
			if ((e.data.foe.getX() - before) * (fm.isFacingRight() ? 1 : -1) > 0) hs.reversibleAngle = false;
		}
		me.__hitFoes.push(e.data.foe);
		// always fresh, and a place among its owner's last attacks once (Attacks.freshness), dealt
		// on top once the hit has landed, as a fighter's is
		me.__freshAdd = { foe: e.data.foe, add: hs.damage * 0.05 };
		if (me.__usedOnce != true) { me.__usedOnce = true; Attacks.used(o, null); }
		if (hs.index != null && hs.index >= 0) Hitboxes.poison(me, hs.index, e.data.foe);
		// a hit in the frame that made it lands before its first frame's script has run, which is
		// where its code listens for its own hits (ness's pk fire turns into its pillar there): the
		// source runs that script first, so the hit is told to those listeners once they are there
		if (PeptideRuntime.frame == 0) { me.__earlyHit = e; me.__earlyFrom = e.data.foe.getDamage(); }
	}, { persistent: true });
	// nothing in this engine ends one that has gone to DESTROYING (its update stops there), so it
	// watches itself on a timer the engine runs regardless, and goes once its death has played
	me.__dyingFor = 0;
	// (every callback the engine makes asks first whether the object is still there, with nothing
	// but its own locals: once it is gone its script answers no names)
	fm.addTimer(1, -1, function(?t) { if (!fm.isDisposed()) Projectiles.dying(me); }, { persistent: true });
};

// at most `limit` of one projectile out at once per owner: the oldest goes (limitOverwrite) or
// the new one does
Projectiles.limit = function(me) {
	var lim = Projectiles.num(me, "limit", 0);
	if (lim < 1) return;
	var fm = me.__fm;
	var owner = fm.getOwner();
	if (owner == null) return;
	// kept on the owner, by this projectile's clip (a map: a record here takes no field by a
	// name worked out as it runs)
	if (owner.exports.__limits == null) owner.exports.__limits = new haxe.ds.StringMap();
	var key = AS3.str(PeptideRuntime.data.object.clip);
	var list = owner.exports.__limits.get(key);
	if (list == null) { list = []; owner.exports.__limits.set(key, list); }
	var k = 0;
	while (k < list.length) { if (list[k] == null || list[k].isDisposed()) list.splice(k, 1); else k++; }
	if (AS3.truthy(Projectiles.stat(me, "limitOverwrite"))) {
		while (list.length >= lim) { var o = list.shift(); if (o != null && !o.isDisposed()) o.kill(); }
		list.push(fm);
	} else if (list.length >= lim) {
		fm.kill();
	} else {
		list.push(fm);
	}
};

// every source frame, before its own code
Projectiles.frame = function(me) {
	var fm = me.__fm;
	if (fm.isDisposed() || fm.getState() == PState.DESTROYING) return;
	// its lifetime (a held item is not flying, so its life does not run)
	// (an enemy riding a projectile lives as long as its own code keeps it: the default lifetime
	// is a projectile's, and pac-man's hydrant went a hundred frames early on it)
	var enemy = fm.exports.__isEnemy == true;
	var life = enemy && (me.__stats == null || !me.__stats.exists("time_max")) ? 0 : Projectiles.num(me, "time_max", 0);
	if (fm.exports.__holder == null) me.__age += 0.5;
	if (life > 0 && me.__age >= life) { Objects.remove(me); return; }
	// the source tells a projectile's floor listeners the first frame it finds it on the floor,
	// one made there too; this engine only tells one that comes down onto it, and only lands one
	// that comes down (pikachu's jolt is made at his feet and starts its hops from that touch;
	// mega man's water wave lands where it is made and makes the next one from its landing)
	if (me.__floorChecks == null) me.__floorChecks = 0;
	if (me.__floorChecks < 2) {
		me.__floorChecks++;
		// made this frame, the engine has not yet put it on anything: a floor through its feet is
		// the floor the source finds it on (pikachu's jolt hops off it the frame it is made)
		var on = fm.isOnFloor() || (me.__floorChecks == 1 && Projectiles.floorAt(fm));
		if (on && Events.touchedFloor(me)) {
			me.__floorChecks = 2;
		}
	}
	// a landing it is given, once: the engine's own when it comes down, or here when it is
	// already on the floor
	if (me.__landsTo != null && me.__landed != true) {
		if (fm.getAnimation() == me.__landsTo) me.__landed = true;
		else if (fm.isOnFloor()) { me.__landed = true; fm.playAnimation(me.__landsTo); }
	}
	Projectiles.trail(me);
	Projectiles.outOfBounds(me);
	// an attack of its own another script asked it to start
	var a = fm.exports.__askAttack;
	if (a != null) { fm.exports.__askAttack = null; PeptideGameObject.startAttack(me, a); }
	// an attack its holder began, put to its frame interrupt (an item's own say in the move)
	var q = fm.exports.__askInterrupt;
	if (q != null) {
		fm.exports.__askInterrupt = null;
		if (me.__frameInterrupt != null) { q.character = Objects.view(q.character); PeptideRuntime.run(me.__frameInterrupt, q); }
	}
	if (!Projectiles.hasOwnCode(me)) Projectiles.defaultEnds(me);
};

// a floor under its feet, by a short line through them
Projectiles.floorAt = function(fm) {
	if (fm.hitTestStructuresWithLineSegment == null) return false;
	var a = EnginePoint.create(fm.getX(), fm.getY() - 2);
	var b = EnginePoint.create(fm.getX(), fm.getY() + 2);
	var hits = fm.hitTestStructuresWithLineSegment(a, b, EnginePoint.create(b.x, b.y), { structureType: StructureType.FLOOR });
	return hits != null && hits.length > 0;
};

// every engine frame
Projectiles.engineFrame = function(me) {
	var fm = me.__fm;
	if (fm.isDisposed() || fm.getState() == PState.DESTROYING) return;
	for (st in me.__selfPlatforms) { st.setX(fm.getX()); st.setY(fm.getY()); }
	Projectiles.refresh(me);
	Attacks.carry(me);
	Projectiles.lateHit(me);
	Projectiles.held(me);
	if (fm.exports.__holder != null) return;
	// what it was made with only answers a touch as it is made (Events.handler): one later is a landing
	if (me.__age != null && me.__age >= 1) fm.exports.__launched = null;
	if (fm.exports.__floorSet != null) { var b = fm.exports.__floorSet; fm.exports.__floorSet = null; fm.setXSpeed(b.xs); fm.setYSpeed(b.ys); }
	// the speed it falls at, for the code it tells of its touchdown (Events.handler): the engine
	// has stopped it by then, and the source tells it with the speed it came down at
	if (!fm.isOnFloor()) fm.exports.__airSpeed = { xs: fm.getXSpeed(), ys: fm.getYSpeed() };
	var u = PeptideRuntime.data.units;
	var xdecay = Projectiles.num(me, "xdecay", 0) * u.speed;
	var homing = Projectiles.num(me, "homingSpeed", -1);
	// a homing one flies at its homing speed less a frame's decay, set anew each frame (its own
	// code can feed the speed it is left with back in, and slows a whole decay a source frame);
	// any other loses the decay as it goes, half a step an engine frame
	// its first source frame flies at its launch (samus's missile leaves at its launch speed and
	// its own code feeds that speed back in as its homing speed by the next)
	if (homing >= 0 && me.__age != null && me.__age >= 1) Projectiles.home(me, homing * u.speed - xdecay);
	else if (xdecay != 0) {
		// a decay slows it to a stop and holds it there, as the source's does (sora's strike raid
		// stops and hangs where it slowed to, its fighter caught under it); one stated below zero
		// speeds it up (din's fire)
		var v = fm.getXSpeed();
		var w = v - xdecay * 0.5;
		fm.setXSpeed(xdecay > 0 && v >= 0 && w < 0 ? 0 : w);
	}
	Projectiles.touchOwner(me);
	Projectiles.influence(me);
	Projectiles.control(me);
	Projectiles.rotate(me);
};

// a hit it landed before its code had started, told to what its code listens with once it has
Projectiles.lateHit = function(me) {
	var e = me.__earlyHit;
	if (e == null || PeptideRuntime.frame < 1) return;
	me.__earlyHit = null;
	// only a hit that landed: one that connected can still be turned off before it deals anything
	// (pikachu's jolt touches the fighter as it is made, and lands nothing there)
	if (me.__handlers == null || e.data.foe.isDisposed() || e.data.foe.getDamage() <= me.__earlyFrom) return;
	var dealt = PeptideRuntime.data.engineEvent("attackConnect");
	for (x in me.__handlers.copy()) if (PeptideRuntime.data.engineEvent(AS3.str(x.type)) == dealt) x.h(e);
};

// the attack it is in hits again every `refreshRate` of the source's frames its declaration states
// for it (ness's fire pillar burns a fighter standing in it every eight), as a fighter's does
Projectiles.refresh = function(me) {
	var label = me.__mc == null ? null : me.__mc.currentLabel;
	var name = label != null && me.__stats != null && me.__stats.get(label) != null ? label : "attack_idle";
	if (name != me.__refreshOf) {
		me.__refreshOf = name;
		me.__refreshAt = 0;
		var a = me.__stats == null ? null : me.__stats.get(name);
		var r = a == null ? null : AS3.at(a, "refreshRate");
		me.__refreshEvery = r != null && AS3.num(r) > 0 ? Std.int(AS3.num(r) * 2) : 0;
	}
	// its code can change the rate as it plays (naruto's rasenshuriken puts it out of reach as it
	// bursts, and hit once there, not every three frames)
	var o = Attacks.overrides(me);
	var every = o.exists("refreshRate") ? (AS3.num(o.get("refreshRate")) > 0 ? Std.int(AS3.num(o.get("refreshRate")) * 2) : 0) : me.__refreshEvery;
	// none stated, -1, or 999 and up: a rate the source never refreshes on
	if (every <= 0 || every >= 999 * 2) return;
	// on its Nth source frame, before that frame's hits, and every N after, as a fighter's attack
	// (AttackRules.refresh): here the first engine frame of that source frame (naruto's
	// rasenshuriken ground the fighter a frame behind the source's, every hit after its first)
	me.__refreshAt++;
	if (me.__refreshAt >= every - 1) { me.__refreshAt = -1; PeptideGameObject.reactivate(me); }
};

// homing: the heading turns toward the middle of the nearest opponent that can be hit, a share of the turn
// each frame (homingEase, per source frame: half as much a frame here), at homingSpeed
Projectiles.home = function(me, speed) {
	var fm = me.__fm;
	var target = null;
	var best = 1.0e12;
	for (c in match.getCharacters()) {
		if (c == fm.getOwner() || c.isDisposed() || c.hasBodyStatus(BodyStatus.INTANGIBLE) || c.hasBodyStatus(BodyStatus.INVINCIBLE)) continue;
		var dx = c.getX() - fm.getX();
		var dy = c.getY() - fm.getY();
		var d = dx * dx + dy * dy;
		if (d < best) { best = d; target = c; }
	}
	if (target == null) return;
	me.__homingSpeed = speed;
	var ease = Projectiles.num(me, "homingEase", 1);
	if (ease < 1) ease = 1;
	// at the middle of its body, half its height over its feet (samus's missile, fired at her
	// chest's height, flies level at a fighter as tall as she is)
	var head = target.getType() == EntityType.CHARACTER ? target.getCharacterStat(target.isOnFloor() ? "floorHeadPosition" : "aerialHeadPosition") : null;
	var ty = target.getY() - (head == null ? 0 : AS3.num(head) / 2);
	var want = Math.atan2(ty - fm.getY(), target.getX() - fm.getX());
	if (me.__heading == null) me.__heading = Math.atan2(fm.getYSpeed(), Projectiles.xSpeed(fm));
	var diff = want - me.__heading;
	while (diff > Math.PI) diff -= 2 * Math.PI;
	while (diff < -Math.PI) diff += 2 * Math.PI;
	me.__heading += diff / (ease * 2);
	fm.setXSpeed(fm.flipX(Math.cos(me.__heading) * speed));
	fm.setYSpeed(Math.sin(me.__heading) * speed);
};

// influence: the owner's held stick nudges it, per axis: stepped toward the held way by the
// axis' Factor, back toward nothing when the stick is let go, capped at its Movement, and added
// to where it is on top of its own flight
Projectiles.influence = function(me) {
	var u = PeptideRuntime.data.units;
	var xm = Projectiles.num(me, "influenceXMovement", 0) * u.speed;
	var ym = Projectiles.num(me, "influenceYMovement", 0) * u.speed;
	if (xm <= 0 && ym <= 0) return;
	var fm = me.__fm;
	var o = fm.getOwner();
	if (o == null || o.getHeldControls == null) return;
	var c = o.getHeldControls();
	var step = function(v, f, cap, plus, minus) {
		if (plus) v += f; else if (minus) v -= f;
		else if (v > 0) { v -= f; if (v < 0) v = 0; }
		else if (v < 0) { v += f; if (v > 0) v = 0; }
		if (v > cap) v = cap;
		if (v < -cap) v = -cap;
		return v;
	};
	if (me.__infx == null) { me.__infx = 0.0; me.__infy = 0.0; }
	if (xm > 0) me.__infx = step(me.__infx, Projectiles.num(me, "influenceXFactor", 0) * u.speed, xm, c.RIGHT, c.LEFT);
	if (ym > 0) me.__infy = step(me.__infy, Projectiles.num(me, "influenceYFactor", 0) * u.speed, ym, c.DOWN, c.UP);
	fm.setX(fm.getX() + me.__infx);
	fm.setY(fm.getY() + me.__infy);
};

// controlDirection: the owner aims it with the stick, eight ways, at its launch's x speed; eased
// toward the aim when calcAngle is set, snapped to it otherwise. it starts along the angle stated
Projectiles.control = function(me) {
	var cd = Projectiles.num(me, "controlDirection", -1);
	if (cd < 0) return;
	var fm = me.__fm;
	var u = PeptideRuntime.data.units;
	if (me.__aim == null) {
		me.__aim = cd * Math.PI / 180;
		me.__aimSpeed = Math.abs(AS3.num(Projectiles.declared(PeptideRuntime.data.object.stats, ["xspeed", "xSpeed", "x_speed"])) * u.speed);
	}
	var o = fm.getOwner();
	if (o != null && o.getHeldControls != null) {
		var c = o.getHeldControls();
		var want = null;
		if (c.UP && c.LEFT) want = 2.35619449; else if (c.UP && c.RIGHT) want = 0.78539816;
		else if (c.DOWN && c.LEFT) want = 3.92699082; else if (c.DOWN && c.RIGHT) want = 5.49778714;
		else if (c.UP) want = 1.57079633; else if (c.DOWN) want = 4.71238898;
		else if (c.LEFT) want = 3.14159265; else if (c.RIGHT) want = 0;
		if (want != null) {
			if (Projectiles.num(me, "calcAngle", 0) != 0) {
				var d = want - me.__aim;
				while (d > Math.PI) d -= 2 * Math.PI;
				while (d < -Math.PI) d += 2 * Math.PI;
				me.__aim += d / 6;
			} else me.__aim = want;
		}
	}
	fm.setXSpeed(fm.flipX(Math.cos(me.__aim) * me.__aimSpeed));
	fm.setYSpeed(-Math.sin(me.__aim) * me.__aimSpeed);
};

// a trail: its effect dropped where it is every trailEffectInterval frames, turned along its
// heading when trailEffectRotate is set
Projectiles.trail = function(me) {
	var name = Projectiles.stat(me, "trailEffect");
	if (name == null || name == "") return;
	me.__trailT++;
	var every = Projectiles.num(me, "trailEffectInterval", 1);
	if (every < 1) every = 1;
	if (me.__trailT < every) return;
	me.__trailT = 0;
	var fm = me.__fm;
	// a shared trail plays this engine's global one; one it neither carries nor has a global one
	// for is not drawn (made on a name with no content, the effect draws the projectile itself)
	var n = AS3.str(name);
	var global = PeptideRuntime.data.globalVfx == null ? null : PeptideRuntime.data.globalVfx.get(n);
	if (global == null && PeptideRuntime.data.effects.get(n) == null) { PeptideRuntime.noEffect(n); return; }
	var content = global != null ? "global::vfx.vfx" : fm.getResource().getContent(n);
	if (content == null) return;
	var v = match.createVfx(new VfxStats({ spriteContent: content, animation: global != null ? global : Effects.animationOf(n) }), fm);
	if (v != null && AS3.truthy(Projectiles.stat(me, "trailEffectRotate"))) {
		v.setRotation(Projectiles.turnFor(v, Projectiles.xSpeed(fm), fm.getYSpeed()));
	}
};

// turned to face the way it flies, when its declaration says so. this engine turns an object after
// mirroring it, so one facing left turns the other way round for the same climb (fox's blaster shot,
// flying up and left, was turned down toward the way it flew, and its long thin box took a fighter
// below the line the source's shot follows: his back throw struck 4 times where the source's strikes 2)
Projectiles.rotate = function(me) {
	if (!AS3.truthy(Projectiles.stat(me, "rotate"))) return;
	var fm = me.__fm;
	var vx = fm.getXVelocity();
	var vy = fm.getYVelocity();
	if (vx != 0 || vy != 0) fm.setRotation(Projectiles.turnFor(fm, vx, vy));
};
// the turn that lines an object facing as `fm` does up with a move of (vx, vy)
Projectiles.turnFor = function(fm, vx, vy) {
	var a = Math.atan2(vy, fm.isFacingRight() ? vx : -vx) * 180 / Math.PI;
	return fm.isFacingRight() ? a : -a;
};

Projectiles.bounce = function(me) {
	var u = PeptideRuntime.data.units;
	var b = Projectiles.num(me, "bounce", 0) * u.speed * Math.pow(Projectiles.num(me, "bounce_decay", 1), me.__bounces);
	me.__bounces++;
	if (b < 0.5) { Objects.remove(me); return; }
	me.__fm.setYSpeed(-b);
};

// the death bounds, kept here: the engine takes one off at them with nothing of its own run first
Projectiles.outOfBounds = function(me) {
	if (AS3.truthy(Projectiles.stat(me, "surviveDeathBounds"))) return;
	var fm = me.__fm;
	// a held item goes where its holder goes: the holder's bounds are the ones that count
	if (fm.exports.__holder != null) return;
	// one its owner steers lives on wherever it is steered
	if (Projectiles.num(me, "controlDirection", -1) != -1) return;
	// (the bounds read once for the projectile: they do not move while it flies)
	if (me.__bounds == null) {
		var a = stage == null ? null : stage.getDeathBounds();
		var r = a == null ? null : a.getRectangle();
		if (r == null) return;
		me.__bounds = [a.getX() + r.x, a.getY() + r.y, a.getX() + r.x + r.width, a.getY() + r.y + r.height];
	}
	var b = me.__bounds;
	var x = fm.getX();
	var y = fm.getY();
	if (x < b[0] || x > b[2] || y < b[1] || y > b[3]) Objects.remove(me);
};

// one whose package ships no code for it dies where the source's engine ends it: on the floor,
// or on landing a hit
Projectiles.hasOwnCode = function(me) {
	if (me.__ownCode == null) {
		var root = PeptideRuntime.root;
		var clip = PeptideRuntime.shownClip();
		me.__ownCode = PeptideRuntime.data.object.cls != PeptideProjectile
			|| (root != null && root.__scripts.keys().hasNext())
			|| (clip != null && clip.__scripts.keys().hasNext());
		if (!me.__ownCode) {
			var fm = me.__fm;
			fm.addEventListener(EntityEvent.COLLIDE_FLOOR, function(e) { if (!fm.isDisposed()) Objects.remove(me); }, { persistent: true });
			fm.addEventListener(GameObjectEvent.HIT_DEALT, function(e) { if (!fm.isDisposed()) Objects.remove(me); }, { persistent: true });
		}
	}
	return me.__ownCode;
};
Projectiles.defaultEnds = function(me) {};

// held by a fighter (an item it generated): it rides the holder's item box, touching nothing
Projectiles.held = function(me) {
	var fm = me.__fm;
	var st = fm.exports.__askState;
	if (st == "hold") {
		fm.exports.__askState = null;
		if (me.__ghost == null) me.__ghost = fm.exports.__ghostDeclared == true;
		fm.updateProjectileStats({ ghost: true });
	} else if (st == "toss") {
		fm.exports.__askState = null;
		fm.updateProjectileStats({ ghost: me.__ghost == true });
	}
	var h = fm.exports.__holder;
	if (h == null) return;
	if (h.isDisposed()) { fm.exports.__holder = null; return; }
	var s = PeptideRuntime.data.units.size;
	var bx = h.exports.__ibx == null ? 0 : h.exports.__ibx;
	var by = h.exports.__iby == null ? 0 : h.exports.__iby;
	fm.setX(h.getX() + h.flipX(bx * s));
	fm.setY(h.getY() + by * s);
	fm.setXSpeed(0);
	fm.setYSpeed(0);
	if (h.isFacingRight()) fm.faceRight(); else fm.faceLeft();
};

// its end: told once to whoever listens for it, then it is gone
// a speed its code set is its own

Projectiles.end = function(me) {
	var fm = me.__fm;
	if (me.__ended != true) { me.__ended = true; Events.dispatch(me, "projDestroyed"); Events.dispatch(me, "itemDestroyed"); Events.dispatch(me, "enemyDestroyed"); }
	if (!fm.isDisposed()) fm.kill();
};

// in DESTROYING: gone once its death has played (its last frame, or as many frames as it has)
Projectiles.dying = function(me) {
	var fm = me.__fm;
	if (fm.isDisposed()) return;
	if (fm.getState() != PState.DESTROYING) { me.__dyingFor = 0; return; }
	// still while it dies: the source's end takes it off where it is
	fm.setXSpeed(0);
	fm.setYSpeed(0);
	me.__dyingFor++;
	if (fm.finalFramePlayed() || me.__dyingFor >= fm.getTotalFrames()) Projectiles.end(me);
};

// touching its owner: the source's engine tells a projectile it has met the one who threw it (a
// boomerang caught on its way back), once as they meet, again only after they part
Projectiles.touchOwner = function(me) {
	if (!Events.has(me, "projCollide") && !Events.has(me, "projCollideOwner")) return;
	var fm = me.__fm;
	var owner = fm.getOwner();
	if (owner == null || owner.isDisposed()) return;
	var hits = fm.collisionTest(owner, CollisionBoxType.HIT, CollisionBoxType.HURT, true);
	if (hits == null || hits.length == 0) hits = fm.collisionTest(owner, CollisionBoxType.HURT, CollisionBoxType.HURT, true);
	var touching = hits != null && hits.length > 0;
	if (touching && me.__touching != true) {
		var o = Objects.view(owner);
		var rec = { receiver: o, opponent: o, caller: me, owner: o };
		Events.dispatch(me, "projCollide", rec);
		Events.dispatch(me, "projCollideOwner", rec);
	}
	me.__touching = touching;
};

// class PeptideProjectile extends PeptideGameObject
var PeptideProjectile = { name: "PeptideProjectile" };
PeptideProjectile.build = function(me) {
	PeptideGameObject.build(me);
	me.__is.push("PeptideProjectile");
	me.__type = "SSF2Projectile";
};
PeptideProjectile.updateProjectileStats = function(me, o) {
	// kept as the projectile's own; the one this engine also keeps is handed on
	for (k in AS3.keysOf(o)) {
		var v = AS3.at(o, k);
		me.__stats.set(k, v);
		// this engine's ghost lets go of the floor too: one that rides the ground (`rideGround`) and
		// is on it stays on it, as the source's does (isaac's gaia turns ghost as it rises from the
		// floor and fell through it here)
		if (k == "ghost") {
			var keep = AS3.truthy(v) && AS3.truthy(Projectiles.stat(me, "rideGround")) && me.__fm.isOnFloor();
			me.__fm.updateProjectileStats({ ghost: AS3.truthy(v) && !keep });
		}
		if (k == "gravity") me.__fm.updateProjectileStats({ gravity: AS3.num(v) * PeptideRuntime.data.units.accel });
		// the fall it is held to: 0 holds it where it is (ness's pk flash bursts and hangs there, and
		// fell on here, its code's cap never reaching the engine)
		if (k == "maxgravity" || k == "max_gravity") me.__fm.updateProjectileStats({ terminalVelocity: AS3.num(v) * PeptideRuntime.data.units.speed });
	}
};
// its gravity, and whether it carries on when it meets the floor or a wall: its own stats (a
// projectile with code of its own ends where its code says, so these only answer it)
PeptideProjectile.modifyGravity = function(me, ?g) { PeptideProjectile.updateProjectileStats(me, { gravity: g }); };
PeptideProjectile.modifyContinueOnGround = function(me, ?on) { me.__stats.set("continueOnGround", on); };
PeptideProjectile.modifyContinueOnCrash = function(me, ?on) { me.__stats.set("continueOnCrash", on); };
PeptideProjectile.exportStats = function(me) {
	var out = {};
	for (k in me.__stats.keys()) AS3.putKey(out, k, me.__stats.get(k));
	return out;
};
PeptideProjectile.angleControl = function(me, speed, angle) {
	var u = PeptideRuntime.data.units;
	var a = AS3.num(angle) * Math.PI / 180;
	me.__fm.setXSpeed(me.__fm.flipX(Math.cos(a) * AS3.num(speed) * u.speed));
	me.__fm.setYSpeed(-Math.sin(a) * AS3.num(speed) * u.speed);
};
PeptideProjectile.init = function(me, ?api) {};
// gone from the match, as any object its code ends
PeptideProjectile.destroy = function(me) { Objects.destroy(me); };
