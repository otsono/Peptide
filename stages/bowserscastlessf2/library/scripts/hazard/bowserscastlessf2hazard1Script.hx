// Thwomp — 1:1 from the SSF2 disasm (stage spawn cycle + Thwomp class), frame-doubled.
// Native HIT_BOXes (HitboxStats: two half boxes, angles 135/45) damage on contact.

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
	ENTRANCE: _prepLocalState("entrance"),
	IDLE: _prepLocalState("idle"),
	FALL: _prepLocalState("fall")
};

// SSF2 constants, frame-doubled / velocity-converted (see the header comment).
var COLUMNS = [293.6, 445.7, 597.8, 950.1, 1102.2, 1254.3];
var LAND_YS = [692.0, 692.0, 692.0, 692.0, 692.0, 692.0];
var SPAWN_Y = -7.1;
var SPAWN_PERIOD = 1200;
var ENTRANCE_T = 120;
var FALL_V = 19.50;
var LAND_WAIT = 180;
var RISE_V = 3.90;
// persistent state (a plain var resets every frame on a custom game object).
var m_phase = self.makeInt(0);
var m_col = self.makeInt(0);
var m_timer = self.makeInt(0);
var m_cycle = self.makeInt(0);
var m_cool = self.makeInt(0);
var m_init = self.makeBool(false);

function initialize() {
	self.setState(PState.ACTIVE);
	Common.toLocalState(LState.ENTRANCE);
}

function update() {
	// match start: park at the spawn point; SSF2's first spawn lands at t=300f (=600 FM),
	// so pre-advance the spawn clock by half a period.
	if (!m_init.get()) { m_init.set(true); self.setX(COLUMNS[0]); self.setY(SPAWN_Y); m_phase.set(0); m_cycle.set(SPAWN_PERIOD - 600); }
	if (m_cool.get() > 0) { m_cool.set(m_cool.get() - 1); } else { self.reactivateHitboxes(); m_cool.set(60); }
	// spawn-to-spawn clock: SSF2 spawns every 600f (=1200 FM) regardless of phase timing.
	m_cycle.set(m_cycle.get() + 1);
	var p = m_phase.get();
	if (p == 0) { // resting between spawns (parked at the spawn point above the stage)
		if (m_cycle.get() >= SPAWN_PERIOD) {
			m_col.set(Random.getInt(0, COLUMNS.length - 1)); self.setX(COLUMNS[m_col.get()]); self.setY(SPAWN_Y);
			m_phase.set(1); m_timer.set(0); m_cycle.set(0); Common.toLocalState(LState.ENTRANCE);
		}
	} else if (p == 1) { // entrance: hover at the spawn point (SSF2 delayTimer 60f)
		m_timer.set(m_timer.get() + 1);
		if (m_timer.get() >= ENTRANCE_T) { m_phase.set(2); Common.toLocalState(LState.FALL); }
	} else if (p == 2) { // fall: constant terminal velocity (gravity 30 capped at 30)
		self.setY(self.getY() + FALL_V);
		if (self.getY() >= LAND_YS[m_col.get()]) { self.setY(LAND_YS[m_col.get()]); m_phase.set(3); m_timer.set(0); Common.toLocalState(LState.IDLE); match.getCamera().shake(16.9); match.createVfx(new VfxStats({ spriteContent: "global::vfx.vfx", animation: GlobalVfx.DUST_POOF, scaleX: 2.6, scaleY: 2.6 }), self); }
	} else if (p == 3) { // landed: the column platform under it sinks; hold (SSF2 waitTimer 90f)
		m_timer.set(m_timer.get() + 1);
		if (m_timer.get() >= LAND_WAIT) { m_phase.set(4); }
	} else { // rise at SSF2 YSpeed -6 until past the spawn point, then rest
		self.setY(self.getY() - RISE_V);
		if (self.getY() <= SPAWN_Y) { self.setY(SPAWN_Y); m_phase.set(0); m_timer.set(0); }
	}
}
