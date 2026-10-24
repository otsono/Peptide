// A hitbox's stats between the source's fields and this engine's, by the measured factors the
// data carries (DATA.hitboxFields, from mappings/character/hitbox_stats.jsonc). The engine's
// fields are named here one by one: a record here cannot take a field by a name worked out as
// it runs.

var Hitboxes = {};
Hitboxes.row = function(fm) {
	for (r in PeptideRuntime.data.hitboxFields) if (r.fm == fm) return r;
	return null;
};
// the first of the row's source keys the stats state, converted
Hitboxes.from = function(o, fm) {
	var r = Hitboxes.row(fm);
	if (r == null) return null;
	for (k in r.keys) {
		if (k == "weightKB") continue;
		var v = AS3.at(o, k);
		if (v == null) continue;
		var n = AS3.num(v);
		// a freeze stated as no count of frames is the one the source works out from the damage
		if (r.frames && (n < 0 || n == 255) && r.perDamage != null && AS3.at(o, "damage") != null)
			return Math.floor(AS3.num(AS3.at(o, "damage")) * r.perDamage + r.constant) * PeptideRuntime.data.frameRatio;
		if (r.frames) n = (n + r.plus) * PeptideRuntime.data.frameRatio;
		if (r.scale != null) n = n * r.scale;
		return n;
	}
	return null;
};
// the back way: an engine value as the source's
Hitboxes.back = function(v, fm) {
	var r = Hitboxes.row(fm);
	if (v == null || r == null) return v;
	var n = AS3.num(v);
	if (r.scale != null) n = n / r.scale;
	if (r.frames) n = n / PeptideRuntime.data.frameRatio - r.plus;
	return n;
};

// the source's box stats, as this engine's hitbox stats
Hitboxes.toEngine = function(o) {
	var out = {};
	var v = Hitboxes.from(o, "damage"); if (v != null) out.damage = v;
	// the angle as the source writes it (see `fm_angle`): both engines mirror a reversible one only
	// for a victim behind
	var rev = AS3.at(o, "reversableAngle");
	v = Hitboxes.from(o, "angle"); if (v != null) out.angle = v;
	if (rev != null) out.reversibleAngle = AS3.truthy(rev);
	v = Hitboxes.from(o, "baseKnockback"); if (v != null) out.baseKnockback = v;
	v = Hitboxes.from(o, "knockbackGrowth"); if (v != null) out.knockbackGrowth = v;
	// a FIXED hit (`weightKB`) launches by what the source works out from the field, with no
	// growth, as the conversion writes a declared one (mappings/character/hitbox_stats.jsonc)
	var w = AS3.num(AS3.at(o, "weightKB"));
	var fk = PeptideRuntime.data.fixedKnockback;
	if (w > 0 && fk != null) {
		var g = AS3.at(o, "kbConstant");
		var p = AS3.at(o, "power");
		var r = Hitboxes.row("baseKnockback");
		var launch = (w * fk.perUnit + fk.constant) * (g == null ? 1 : AS3.num(g) / 100) + (p == null ? 0 : AS3.num(p));
		out.baseKnockback = r != null && r.scale != null ? launch * r.scale : launch;
		out.knockbackGrowth = 0;
	}
	v = Hitboxes.from(o, "hitstop"); if (v != null) out.hitstop = v;
	v = Hitboxes.from(o, "selfHitstop"); if (v != null) out.selfHitstop = v;
	v = Hitboxes.from(o, "shieldstunMultiplier"); if (v != null) out.shieldstunMultiplier = v;
	// what the box states true or false that this engine has a field for (DATA.hitboxFlags), each
	// field named here since a record takes no field by a name worked out as it runs. last, so a
	// flag that states a field holds over the box's own (a box with no effect launches nothing)
	if (PeptideRuntime.data.hitboxFlags != null) for (r in PeptideRuntime.data.hitboxFlags) {
		var f = AS3.at(o, r.key);
		if (f == null || AS3.truthy(f) != r.when) continue;
		switch (r.fm) {
			case "stackKnockback": out.stackKnockback = r.value;
			case "tumbleType": out.tumbleType = r.value;
			case "directionalInfluence": out.directionalInfluence = r.value;
			case "shieldable": out.shieldable = r.value;
			case "physicsHitConditions": out.physicsHitConditions = r.value;
			case "freezeType": out.freezeType = r.value;
			case "flinch": out.flinch = r.value;
			case "hitstop": out.hitstop = r.value;
			case "baseKnockback": out.baseKnockback = r.value;
			case "knockbackGrowth": out.knockbackGrowth = r.value;
			default:
		}
	}
	return out;
};

// this engine's hitbox stats, as the source's box stats (what an event hands the source)
Hitboxes.fromEngine = function(hs) {
	if (hs == null) return null;
	return { damage: hs.damage, direction: hs.angle, angle: hs.angle,
		power: Hitboxes.back(hs.baseKnockback, "baseKnockback"), kbConstant: Hitboxes.back(hs.knockbackGrowth, "knockbackGrowth"),
		hitStun: Hitboxes.back(hs.hitstop, "hitstop"), priority: 1 };
};

// a write the source makes to one of its attack's boxes (numbered from 1): kept for it to read
// back, and put on the engine's box of the same place (numbered from 0)
// a converted write laid onto a hit's own stats, field by field (the engine's record of the hit)
Hitboxes.onto = function(hs, t) {
	if (t.damage != null) hs.damage = t.damage;
	if (t.angle != null) hs.angle = t.angle;
	if (t.baseKnockback != null) hs.baseKnockback = t.baseKnockback;
	if (t.knockbackGrowth != null) hs.knockbackGrowth = t.knockbackGrowth;
	if (t.hitstop != null) hs.hitstop = t.hitstop;
	if (t.selfHitstop != null) hs.selfHitstop = t.selfHitstop;
	if (t.reversibleAngle != null) hs.reversibleAngle = t.reversibleAngle;
	if (t.directionalInfluence != null) hs.directionalInfluence = t.directionalInfluence;
	if (t.stackKnockback != null) hs.stackKnockback = t.stackKnockback;
	if (t.tumbleType != null) hs.tumbleType = t.tumbleType;
	if (t.physicsHitConditions != null) hs.physicsHitConditions = t.physicsHitConditions;
	if (t.freezeType != null) hs.freezeType = t.freezeType;
	if (t.flinch != null) hs.flinch = t.flinch;
	if (t.shieldable != null) hs.shieldable = t.shieldable;
	if (t.shieldstunMultiplier != null) hs.shieldstunMultiplier = t.shieldstunMultiplier;
};
Hitboxes.update = function(me, i, stats) {
	var n = AS3.int(i) - 1;
	if (n < 0) n = 0;
	me.__fm.updateHitboxStats(n, Hitboxes.toEngine(Hitboxes.withFreezes(me, i, stats)));
};
// a write of a box's damage, with the freezes the box does not state: the source works those out
// from the damage it hits with, so a new damage moves them (wario's side special writes 12 over its
// stated 8, and holds wario 3 frames where the 8 held 2)
Hitboxes.withFreezes = function(me, i, stats) {
	if (AS3.at(stats, "damage") == null) return stats;
	var worked = [];
	for (r in PeptideRuntime.data.hitboxFields) {
		if (!r.frames || r.perDamage == null || r.keys.length == 0) continue;
		var k = r.keys[0];
		if (AS3.at(stats, k) != null) continue;
		var v = Attacks.boxStat(me, i, k);
		if (v == null || AS3.num(v) < 0 || AS3.num(v) == 255) worked.push(k);
	}
	if (worked.length == 0) return stats;
	return { __get: function(k) { var v = AS3.at(stats, k); return v == null && worked.indexOf(k) >= 0 ? -1 : v; } };
};

// a box that poisons (`poison` damage every `poisonInterval` of the source's frames, for
// `poisonLength` of them): the one it hits takes it as it goes (black mage's down throw ticks one
// a quarter second for nine). this engine has no poison of its own to state on a box
Hitboxes.poison = function(me, index, foe) {
	var p = Attacks.boxStat(me, index + 1, "poison");
	if (p == null || !(AS3.num(p) > 0) || foe == null) return;
	var every = AS3.num(Attacks.boxStat(me, index + 1, "poisonInterval"));
	var length = AS3.num(Attacks.boxStat(me, index + 1, "poisonLength"));
	if (!(every > 0) || !(length > 0)) return;
	var dmg = AS3.num(p);
	var ticks = Std.int(length / every);
	// one poison on a fighter at a time: a new one takes the place of the one before
	var token = {};
	foe.exports.__poison = token;
	me.__fm.addTimer(Std.int(every * 2), ticks, function(?t) { if (!foe.isDisposed() && foe.exports.__poison == token) foe.addDamage(dmg); }, { persistent: true });
};
