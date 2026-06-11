// Stage hazard (custom game object) — converted from SSF2.
// Local state machine (clean multi-animation on a non-character entity) + the native
// hitbox (HitboxStats). null owner is fine for damage. `motion` = the SSF2 movement.

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
	ACTIVE: _prepLocalState("gameObjectIdle"),
	INACTIVE: _prepLocalState("gameObjectInactive")
};

var REHIT = 30;
// persistent state (a plain var resets every frame on a custom game object).
var m_frame = self.makeInt(0);
var m_baseX = self.makeFloat(0.0);
var m_baseY = self.makeFloat(0.0);
var m_init = self.makeBool(false);
var m_cooldown = self.makeInt(0);

function initialize() {
	self.setState(PState.ACTIVE);
	Common.toLocalState(LState.ACTIVE);
}

function update() {
	if (!m_init.get()) { m_baseX.set(self.getX()); m_baseY.set(self.getY()); m_init.set(true); }
	m_frame.set(m_frame.get() + 1);
	// re-arm the native HIT_BOX so a fighter standing in the hazard keeps taking hits
	// (a hitbox hits each target once per attack id; reactivateHitboxes issues a fresh one).
	if (Common.inLocalState(LState.ACTIVE)) {
		if (m_cooldown.get() > 0) { m_cooldown.dec(); }
		else { self.reactivateHitboxes(); m_cooldown.set(REHIT); }
	}
}
