// The fighter helpers content extends instead of the plain fighter (PeptideCharacterExt): directions by
// facing, landing lag that runs on into a move's `continue` label and a list of effects cleared when
// the fighter's state changes.

// class PeptideCharacterExt extends PeptideCharacter
var PeptideCharacterExt = { name: "PeptideCharacterExt" };
PeptideCharacterExt.build = function(?me) {
	PeptideCharacter.build(me);
	me.__is.push("PeptideCharacterExt");
	me.effects = [];
	me.clearListener = false;

};
// on the ground, a move with landing lag carries on at its `continue` label rather than landing
PeptideCharacterExt.jumpToContinue = function(me, ?e) {
	Events.remove(me, "groundTouch", PeptideCharacterExt.on(me, "jumpToContinue", PeptideCharacterExt.jumpToContinue));
	Attacks.update(me, ({ allowControl: false, cancelWhenAirborne: true }));
	PeptideGameObject.playFrame(me, "continue");
};
PeptideCharacterExt.__land = function(me, ?e) { me.__fm.toState(CState.LAND); };
PeptideCharacterExt.setLandingLag = function(me, ?on) {
	var land = AS3.truthy(on) ? PeptideCharacterExt.on(me, "jumpToContinue", PeptideCharacterExt.jumpToContinue) : PeptideCharacterExt.on(me, "__land", PeptideCharacterExt.__land);
	var other = AS3.truthy(on) ? PeptideCharacterExt.on(me, "__land", PeptideCharacterExt.__land) : PeptideCharacterExt.on(me, "jumpToContinue", PeptideCharacterExt.jumpToContinue);
	Events.remove(me, "groundTouch", other);
	Events.add(me, "groundTouch", land, { persistent: true });
	if (me.__fm.isOnFloor()) land();
};
// effects the move made, taken off together
PeptideCharacterExt.addEffectToList = function(me, ?fx) {
	if (fx == null) { PeptideRuntime.say("an empty effect was added to the list"); return null; }
	me.effects.push(fx);
	return fx;
};
PeptideCharacterExt.removeAllEffects = function(me, ?e) {
	for (fx in me.effects) if (fx != null && fx.parent != null) fx.parent.removeChild(fx);
	me.effects = [];
	var listening = Events.has(me, "stateChange", PeptideCharacterExt.on(me, "removeAllEffects", PeptideCharacterExt.removeAllEffects));
	if ((AS3.truthy(me.clearListener) && listening) || e != null) Events.remove(me, "stateChange", PeptideCharacterExt.on(me, "removeAllEffects", PeptideCharacterExt.removeAllEffects));
};
PeptideCharacterExt.clearEffectsOnStateChange = function(me, ?once) {
	me.clearListener = once;
	Events.add(me, "stateChange", PeptideCharacterExt.on(me, "removeAllEffects", PeptideCharacterExt.removeAllEffects), { persistent: true });
};
PeptideCharacterExt.stopListening = function(me) {
	me.clearListener = false;
	Events.remove(me, "stateChange", PeptideCharacterExt.on(me, "removeAllEffects", PeptideCharacterExt.removeAllEffects));
};
PeptideCharacterExt.pushEffectBehind = function(me, ?fx) {
	Stage.layer.swapChildren(me.__mc, fx);
	return fx;
};
// autolink is the engine's own, set up at conversion from the move's clip (compile/autolink.rs);
// one started from code outside the move's clip has nothing to set up here
PeptideCharacterExt.startAutolink = function(me, ?point, ?rule, ?maxPower) { PeptideRuntime.gap("autolink started outside its move's clip"); };
PeptideCharacterExt.stopAutolink = function(me) {};
PeptideCharacterExt.setAutolinkPoint = function(me, ?point) {};
PeptideCharacterExt.setAutolinkRule = function(me, ?rule) {};
// one of these helpers as a listener: the same function each time for the same fighter, so a
// listener added with it is removed with it (content taking the member by name gets this one too)
PeptideCharacterExt.on = function(me, n, f) {
	return PeptideRuntime.bound(me, n, function(?a0, ?a1, ?a2, ?a3) { return f(me, a0, a1, a2, a3); });
};
PeptideCharacterExt.init = function(?me, ?api) { PeptideCharacter.init(me, api); };
PeptideCharacterExt.create = function(?api) { var me = {}; PeptideCharacterExt.build(me); PeptideCharacterExt.init(me, api); return me; };
