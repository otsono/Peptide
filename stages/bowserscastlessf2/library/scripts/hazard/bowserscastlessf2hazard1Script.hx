// Thwomp reconstructed from its SSF2 update()/initialize() via the character decompiler,
// then made FM-CGO-runnable: field state -> local states + self.make* slots, FrameTimers ->
// counters, engine physics -> a scripted kinematics integrator (30fps units converted), and
// the STAGE class's spawn cycle synthesized around the enemy class's own state machine.


// stage spawn machine constants (stepped from the stage + enemy classes)
var COLUMNS = [293.6, 445.7, 597.8, 950.1, 1102.2, 1254.3];
var LAND_YS = [692.0, 692.0, 692.0, 692.0, 692.0, 692.0];
var SPAWN_Y = -7.1;
var SPAWN_PERIOD = 1200;
var TERMINAL_V = 19.50;
var _w_init = self.makeBool(false);
var _w_active = self.makeBool(false);
var _w_col = self.makeInt(0);
var _w_clock = self.makeInt(600);
var _w_cool = self.makeInt(0);
var _w_prev = self.makeInt(-99);
var _w_state_t = self.makeInt(0);
var _kin_vy = self.makeFloat(0.0);
var _kin_grav = self.makeFloat(0.0);

// the SSF2 engine's isOnFloor: resting on the spawn column's landing surface.
function __onFloor():Bool {
	return self.getY() >= LAND_YS[_w_col.get()];
}

function __hazardInit() {
	// [SSF2-only: forceAttack] self.forceAttack("entrance");
	Common.toLocalState(-1);
	// timer init -> persistent counter (below)
	// timer init -> persistent counter (below)
	// [SSF2-only: createSelfPlatform] self.m_selfPlatform = self.createSelfPlatform(-55, -120, 120, 130);
	// [needs-port] self.m_selfPlatform.setFallthrough(true);
	// [needs-port] self.setCamBoxSize(110 + 30, 130 + 30, -15, -15);
	match.getCamera().addTarget(self);
	return;
}


function __hazardUpdate() {
	var _v1 = null;
	_v1 = null;
	// --------- SUBSTATE SYSTEM ----------
	if (Common.inLocalState(-1)) {
		// FrameTimer tick -> the frames-in-state counter
		if ((_w_state_t.get() >= (60) * 2)) {
			Common.toLocalState(0);
			_kin_grav.set(9.75); // SSF2 gravity 30 @30fps -> FM accel
			// [needs-port] self.m_selfPlatform.setFallthrough(false);
		}
	} else if (Common.inLocalState(0)) {
		if (__onFloor()) {
			Common.toLocalState(1);
			// [SSF2-only: forceAttack] self.forceAttack("idle");
			// [needs-port] AudioClip.play("thwomp_land");
			// [needs-port] AudioClip.play("thwomp_vfx");
			match.createVfx(new VfxStats({ spriteContent: "global::vfx.vfx", animation: GlobalVfx.DUST_POOF, scaleX: 2.6, scaleY: 2.6 }), self);
			match.getCamera().shake(16.9);
			// [SSF2-only: cast] _v1 = SSF2Utils.cast(self.getCurrentPlatform(), BowsersCastlePlatform);
			// [SSF2-only: cast] if (SSF2Utils.cast(self.getCurrentPlatform(), BowsersCastlePlatform)) {
				// [SSF2-dead] match.shakeCamera(13);
				// [SSF2-dead] _v1.sink();
			// [SSF2-dead] }
		}
	} else if (Common.inLocalState(1)) {
		// FrameTimer tick -> the frames-in-state counter
		if ((_w_state_t.get() >= (30 * 3) * 2)) {
			Common.toLocalState(2);
			// [SSF2-only: unnattachFromGround] self.unnattachFromGround();
			_kin_grav.set(0.00); // SSF2 gravity 0 @30fps -> FM accel
			_kin_vy.set(-3.90); // SSF2 setYSpeed -6 @30fps
			// FrameTimer reset -> frames-in-state resets on transition
		}
	} else if (Common.inLocalState(2)) {
	}
}


function update() {
	// one-time setup on the first update (the engine doesn't call initialize() on a stage
	// CGO, and make* slots aren't live at module scope): register states + park offscreen.
	if (!_w_init.get()) {
		_w_init.set(true);
		self.setState(PState.ACTIVE);
		Common.initLocalStateMachine();
		Common.registerLocalState(-1, "entrance");
		Common.registerLocalState(0, "fall");
		Common.registerLocalState(1, "idle");
		Common.registerLocalState(2, "idle");
		self.setX(COLUMNS[0]);
		self.setY(SPAWN_Y);
	}
	// spawn-to-spawn clock (the stage spawn machine runs regardless of the enemy's phase)
	_w_clock.inc();
	if (!_w_active.get()) {
		if (_w_clock.get() >= SPAWN_PERIOD) {
			_w_clock.set(0);
			_w_col.set(Random.getInt(0, COLUMNS.length - 1));
			self.setX(COLUMNS[_w_col.get()]);
			self.setY(SPAWN_Y);
			_kin_vy.set(0);
			_kin_grav.set(0);
			_w_prev.set(-99);
			__hazardInit();
			_w_active.set(true);
		}
		return;
	}
	// frames-in-state: every SSF2 FrameTimer here measures time since entering its state
	if (Common.getLocalState() != _w_prev.get()) {
		_w_prev.set(Common.getLocalState());
		_w_state_t.set(0);
	} else {
		_w_state_t.inc();
	}
	if (_w_cool.get() > 0) {
		_w_cool.dec();
	} else {
		self.reactivateHitboxes();
		_w_cool.set(60);
	}
	// entrance bob: the entrance sub-clip's frame scripts (setYSpeed timeline, 30->60fps)
	if (Common.inLocalState(-1)) {
		if (_w_state_t.get() == 0) {
			_kin_vy.set(5.20);
		}
		if (_w_state_t.get() == 58) {
			_kin_vy.set(0.00);
		}
		if (_w_state_t.get() == 112) {
			_kin_vy.set(-2.60);
		}
	}
	__hazardUpdate();
	// kinematics integrator: the SSF2 engine's gravity/yspeed step, 30fps units converted
	if (_kin_grav.get() > 0 && _kin_vy.get() < TERMINAL_V) {
		_kin_vy.set(Math.min(_kin_vy.get() + _kin_grav.get(), TERMINAL_V));
	}
	if (_kin_vy.get() > 0 && self.getY() + _kin_vy.get() >= LAND_YS[_w_col.get()]) {
		self.setY(LAND_YS[_w_col.get()]); // the engine lands it on the column surface
		_kin_vy.set(0);
	} else {
		self.setY(self.getY() + _kin_vy.get());
	}
	// SSF2 culls it past the death bounds (surviveDeathBounds=false); recycle for the next spawn
	if (_kin_vy.get() < 0 && self.getY() <= SPAWN_Y) {
		self.setY(SPAWN_Y);
		_kin_vy.set(0);
		match.getCamera().deleteTarget(self);
		_w_active.set(false);
	}
}