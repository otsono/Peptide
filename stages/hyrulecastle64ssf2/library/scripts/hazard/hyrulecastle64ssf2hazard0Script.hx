// Animated stage hazard (custom game object) — converted from SSF2. Local state machine
// across the labelled animations; native HIT_BOX (HitboxStats) on the active ones.
// Cross-frame state via self.make* (a plain var re-inits every frame on a game object).

function _prepLocalState(animation:String, ?index:Int=Math.NaN):Int {
	if (!__hasInitLocalStateMachine) { Common.initLocalStateMachine(); __hasInitLocalStateMachine = true; }
	if (index != Math.NaN) { index = __localStatePrepIndex++; }
	Common.registerLocalState(index, animation);
	return index;
}
var __hasInitLocalStateMachine = false;
var __localStatePrepIndex = -1;
var LState = {
	UNINITIALIZED: _prepLocalState("#n/a", -1),
	STAND: _prepLocalState("stand")
};

var REHIT = 30;
var m_cooldown = self.makeInt(0);

function initialize() {
	self.setState(PState.ACTIVE);
	Common.toLocalState(LState.STAND);
}

function update() {
	if (m_cooldown.get() > 0) { m_cooldown.set(m_cooldown.get() - 1); }
	else { self.reactivateHitboxes(); m_cooldown.set(REHIT); }
}
