// Thwomp (converted from SSF2). Falls onto a platform column -> that platform sinks; then
// rises and moves to the next column. Native HIT_BOX (HitboxStats) damages on contact.

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

var COLUMNS = [269.0, 421.0, 573.0, 926.0, 1078.0, 1230.0];
var TOP_Y = 224.0;
var LAND_Y = 564.0;
var m_col = 0;
var m_phase = 0;
var m_fallV = 0.0;
var m_timer = 0;
var m_cool = 0;
var m_init = false;

function initialize() {
	self.setState(PState.ACTIVE);
	Common.toLocalState(LState.ACTIVE);
}

function update() {
	if (!m_init) { m_init = true; self.setX(COLUMNS[m_col]); self.setY(TOP_Y); }
	// keep the native hitbox live so it damages fighters it falls through.
	if (m_cool > 0) { m_cool = m_cool - 1; } else { self.reactivateHitboxes(); m_cool = 18; }
	if (m_phase == 0) {
		m_fallV = m_fallV + 0.9;
		self.setY(self.getY() + m_fallV);
		if (self.getY() >= LAND_Y) { self.setY(LAND_Y); m_phase = 1; m_timer = 0; }
	} else if (m_phase == 1) {
		m_timer = m_timer + 1;
		if (m_timer >= 80) { m_phase = 2; }
	} else {
		self.setY(self.getY() - 6.0);
		if (self.getY() <= TOP_Y) { self.setY(TOP_Y); m_phase = 0; m_fallV = 0.0; m_col = (m_col + 1) % COLUMNS.length; self.setX(COLUMNS[m_col]); }
	}
}
