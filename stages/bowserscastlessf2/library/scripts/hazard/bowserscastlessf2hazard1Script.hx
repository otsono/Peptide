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
var m_frame = 0;
var m_baseX = 0.0;
var m_baseY = 0.0;
var m_init = false;
var m_cooldown = 0;

function initialize() {
	self.setState(PState.ACTIVE);
	Common.toLocalState(LState.ACTIVE);
}

function update() {
	if (!m_init) { m_baseX = self.getX(); m_baseY = self.getY(); m_init = true; }
	m_frame = m_frame + 1;
	// re-arm the native HIT_BOX so a fighter standing in the hazard keeps taking hits
	// (a hitbox hits each target once per attack id; reactivateHitboxes issues a fresh one).
	if (Common.inLocalState(LState.ACTIVE)) {
		if (m_cooldown > 0) { m_cooldown = m_cooldown - 1; }
		else { self.reactivateHitboxes(); m_cooldown = REHIT; }
	}
}
