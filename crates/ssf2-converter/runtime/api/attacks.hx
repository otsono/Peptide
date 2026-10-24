// The attack the fighter is in, as its package declares it (aData, by the fighter clip's label),
// with what the source's code changed of it since the attack began.

var Attacks = {};
Attacks.current = function(me) {
	var root = me.__mc;
	if (root == null || root.currentLabel == null) return null;
	var a = AS3.at(PeptideRuntime.data.pkg.attacks, root.currentLabel);
	if (a != null || me.__fm == null || me.__fm.getType() == EntityType.CHARACTER || me.__stats == null) return a;
	// what isn't a fighter keeps its attacks in its own declaration, the one it starts in being its
	// idle one (black mage's meteor reads its own damage to add its charge to it)
	a = me.__stats.get(root.currentLabel);
	return a != null ? a : me.__stats.get("attack_idle");
};
Attacks.overrides = function(me) {
	var root = me.__mc;
	// a fighter's attack is its clip's label; a projectile's is the projectile's own, whatever its
	// clip plays (samus's bomb writes its blast's damage as it goes on from `start` to `continue`)
	// (what kind of object it is, asked of the engine once an object)
	if (me.__kindOf != me.__fm) { me.__kindOf = me.__fm; me.__isProjectile = me.__fm != null && me.__fm.getType() == EntityType.PROJECTILE; }
	var label = root == null ? null : (me.__isProjectile ? "projectile" : root.currentLabel);
	if (me.__atk == null || me.__atkLabel != label) { me.__atkLabel = label; me.__atk = new haxe.ds.StringMap(); me.__atkBox = new haxe.ds.IntMap(); me.__atkWrites = null; me.__atkWritesOf = label; }
	return me.__atk;
};
Attacks.stat = function(me, n) {
	var o = Attacks.overrides(me);
	if (o.exists(n)) return o.get(n);
	var fired = me.__fm == null || me.__fm.exports == null ? null : me.__fm.exports.__fired;
	if (fired != null) { var f = AS3.at(fired, n); if (f != null) return f; }
	// what the package declares for the attack never changes, and the rules ask for it several
	// times a frame: each stat is looked up once a label
	var root = me.__mc;
	var label = root == null ? null : root.currentLabel;
	if (label != null) {
		if (me.__declOf != label) { me.__declOf = label; me.__decl = AS3.at(PeptideRuntime.data.pkg.attacks, label); me.__declSeen = new haxe.ds.StringMap(); }
		if (me.__decl != null) {
			var seen = me.__declSeen;
			if (seen.exists(n)) return seen.get(n);
			var v = AS3.at(me.__decl, n);
			seen.set(n, v);
			return v;
		}
	}
	var a = Attacks.current(me);
	return a == null ? null : AS3.at(a, n);
};
// the source's damage freshness: a fighter keeps the last nine attacks that hit, the latest first.
// one not among them deals 1.05 of its damage; one among them 1 less 0.10, 0.09 ... 0.02 for each
// place it holds. a fighter's move counts once a use, by its attack; a projectile is its own each
// time (always fresh) and still takes a place. measured on the source: krystal's down tilt deals
// 7.4, 6.3, 5.7, 5.1, 4.6, 4.2, 3.9, 3.6, 3.4, 3.2 used ten times over, fresh again after nine jabs,
// 6.4 with one jab between; mario's fireball 5.2 every time, and a down tilt after one 9.1 (this
// engine stales nothing of its own)
Attacks.freshness = function(fm, key) {
	var q = Attacks.usedList(fm);
	var cut = 0.0;
	var any = false;
	var i = 0;
	while (i < q.length) { if (key != null && q[i] == key) { cut += 0.10 - 0.01 * i; any = true; } i++; }
	return any ? 1 - cut : 1.05;
};
// the freshness a hit deals on top of its own damage (worked out as it connects, `__freshAdd`),
// put on the one it hit once it has landed: the launch is the move's own damage's
Attacks.freshDealt = function(me, foe) {
	var f = me.__freshAdd;
	me.__freshAdd = null;
	if (f == null || foe == null || f.foe != foe || !(f.add != 0)) return;
	foe.addDamage(f.add);
};
// what this use of a fighter's move deals by how fresh it is, worked out the first time it deals
// anything and kept for the rest of that use
Attacks.useFresh = function(me) {
	if (me.__freshOf != me.__useSerial || me.__freshOf == null) {
		me.__freshOf = me.__useSerial;
		me.__fresh = Attacks.freshness(me.__fm, me.__rulesBase);
		Attacks.used(me.__fm, me.__rulesBase);
	}
	return me.__fresh;
};
Attacks.used = function(fm, key) {
	var q = Attacks.usedList(fm);
	q.insert(0, key == null ? {} : key);
	if (q.length > 9) q.pop();
};
Attacks.usedList = function(fm) {
	if (fm == null || fm.exports == null) return [];
	if (fm.exports.__usedAttacks == null) fm.exports.__usedAttacks = [];
	return fm.exports.__usedAttacks;
};
Attacks.update = function(me, stats) {
	var o = Attacks.overrides(me);
	for (k in AS3.keysOf(stats)) o.set(k, AS3.at(stats, k));
	// the object's kind acts on what changed (a fighter's motion and steering take it at once)
	if (me == PeptideRuntime.object && PeptideRuntime.kind.attackChanged != null) PeptideRuntime.kind.attackChanged(me, stats);
};
Attacks.boxStat = function(me, i, n) {
	Attacks.overrides(me);
	var b = me.__atkBox.get(i);
	if (b != null && b.exists(n)) return b.get(n);
	var a = Attacks.current(me);
	var boxes = a == null ? null : AS3.at(a, "attackBoxes");
	var box = boxes == null ? null : AS3.at(boxes, i == 1 ? "attackBox" : "attackBox" + AS3.str(i));
	return box == null ? null : AS3.at(box, n);
};
// how long the attack has run, in source frames: 0 as it starts, one more each frame it is not
// held still by a hit (jigglypuff's forward smash lunges by its charge, read off this; it read the
// match's running time here and slid off the stage). what something else asks of its owner
Attacks.execTime = function(me) {
	var o = me.__execFrom == null && me.__fm != null && me.__fm.getType() != EntityType.CHARACTER ? Objects.view(me.__fm.getOwner()) : me;
	if (o == null || o.__execFrom == null || o.__rulesBase == null) return -1;
	var frozen = (o.__frozenFrames == null ? 0 : o.__frozenFrames) - o.__execFrozen;
	return Std.int((PeptideRuntime.frame - o.__execFrom - frozen) / 2);
};
Attacks.updateBox = function(me, i, stats) {
	Attacks.overrides(me);
	var b = me.__atkBox.get(i);
	if (b == null) { b = new haxe.ds.StringMap(); me.__atkBox.set(i, b); }
	for (k in AS3.keysOf(stats)) b.set(k, AS3.at(stats, k));
	if (me.__atkWrites == null) me.__atkWrites = [];
	me.__atkWrites.push({ i: i, stats: stats });
	// a write every frame keeps only its latest ones (each writes the fields it names over the last)
	if (me.__atkWrites.length > 16) me.__atkWrites.shift();
	Hitboxes.update(me, i, stats);
};
// what the code wrote to the attack's boxes holds for the whole attack, through every animation
// its clip plays: this engine's boxes take a write for the animation they are in, and the next
// one starts from its own stats (ness's pk fire pillar, written to 1 damage, hit for 4 in its loop),
// as it does when its boxes come back for another hit
Attacks.carry = function(me, ?again) {
	var anim = me.__fm.getAnimation();
	if (anim == me.__writtenAnim && again != true) return;
	me.__writtenAnim = anim;
	Attacks.overrides(me);
	if (me.__atkWrites == null || me.__atkWritesOf != me.__atkLabel) { me.__atkWrites = null; me.__atkWritesOf = me.__atkLabel; return; }
	for (w in me.__atkWrites) Hitboxes.update(me, w.i, w.stats);
};
