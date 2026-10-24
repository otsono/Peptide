// An attack the source turns off or back on by its name (`setAttackEnabled(false, "b_down")`).
// This engine turns off an ACTION, with a status effect held until it is turned back on. The
// source's name is a label; the actions are the ones its animations start.

var Actions = {};

// this engine's animation names, by the action each starts (the longest name that begins an
// animation's name is its action)
Actions.table = [
	["jab", CharacterActions.JAB], ["dash_attack", CharacterActions.DASH_ATTACK],
	["tilt_forward", CharacterActions.TILT_FORWARD], ["tilt_up", CharacterActions.TILT_UP], ["tilt_down", CharacterActions.TILT_DOWN],
	["strong_forward", CharacterActions.STRONG_FORWARD], ["strong_up", CharacterActions.STRONG_UP], ["strong_down", CharacterActions.STRONG_DOWN],
	["aerial_neutral", CharacterActions.AERIAL_NEUTRAL], ["aerial_forward", CharacterActions.AERIAL_FORWARD], ["aerial_back", CharacterActions.AERIAL_BACK],
	["aerial_up", CharacterActions.AERIAL_UP], ["aerial_down", CharacterActions.AERIAL_DOWN],
	["special_neutral", CharacterActions.SPECIAL_NEUTRAL], ["special_side", CharacterActions.SPECIAL_SIDE], ["special_up", CharacterActions.SPECIAL_UP], ["special_down", CharacterActions.SPECIAL_DOWN],
	["grab", CharacterActions.GRAB]
];

// the actions the animations of the source's label `n` start
Actions.of = function(n) {
	var out = [];
	for (p in PeptideRuntime.data.pieces) {
		if (p.label != n) continue;
		var best = null;
		for (row in Actions.table) {
			if (p.anim.indexOf(row[0]) == 0 && (best == null || row[0].length > best[0].length)) best = row;
		}
		if (best == null) continue;
		var a = best[1];
		if (a != null && out.indexOf(a) < 0) out.push(a);
	}
	return out;
};

// turned off with no count of frames, an attack comes back after the count its attack data
// declares (`reenableTimer`, source frames); 0 keeps it off
Actions.set = function(me, on, n, frames) {
	var fm = me.__fm;
	if (me.__actOff == null) { me.__actOff = []; me.__actEffect = []; }
	if (!AS3.truthy(on) && (frames == null || frames < 0)) {
		var d = AS3.at(AS3.at(PeptideRuntime.data.pkg.attacks, n), "reenableTimer");
		frames = (d != null && d > 0) ? d : null;
	}
	if (!AS3.truthy(on) && frames != null && frames > 0) {
		var back = function() { Actions.set(me, true, n, null); };
		Timers.create(me, frames, 1, back, { persistent: true });
	}
	for (a in Actions.of(n)) {
		var k = me.__actOff.indexOf(a);
		if (AS3.truthy(on) && k >= 0) {
			fm.removeStatusEffect(StatusEffectType.DISABLE_ACTION, me.__actEffect[k]);
			me.__actOff.splice(k, 1);
			me.__actEffect.splice(k, 1);
		} else if (!AS3.truthy(on) && k < 0) {
			var e = fm.addStatusEffect(StatusEffectType.DISABLE_ACTION, a);
			me.__actOff.push(a);
			me.__actEffect.push(e == null ? null : e.id);
		}
	}
};

Actions.enabled = function(me, n) {
	if (me.__actOff == null) return true;
	for (a in Actions.of(n)) if (me.__actOff.indexOf(a) >= 0) return false;
	return true;
};

// the source's engine puts every attack turned off back on when the fighter lands (measured:
// pikachu's side special from a jump refuses his up special until he is down, and gives it back
// once he is)
Actions.frame = function(me) {
	var fm = me.__fm;
	if (!fm.isOnFloor()) { me.__actAirborne = true; return; }
	if (me.__actAirborne != true) return;
	me.__actAirborne = false;
	if (me.__actOff == null) return;
	while (me.__actOff.length > 0) {
		fm.removeStatusEffect(StatusEffectType.DISABLE_ACTION, me.__actEffect[0]);
		me.__actOff.splice(0, 1);
		me.__actEffect.splice(0, 1);
	}
};
