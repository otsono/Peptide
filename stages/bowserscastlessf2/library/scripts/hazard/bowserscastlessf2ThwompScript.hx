// Thwomp reconstructed from its SSF2 update()/initialize() via the character decompiler,
// then made FM-CGO-runnable: field state -> named local states, FrameTimers -> the engine's
// makeFrameTimer, physics -> a scripted kinematics integrator (30fps units converted), and
// the STAGE class's spawn cycle synthesized around the enemy class's own state machine.

var ST_ENTRANCE = -1;
var ST_FALL = 0;
var ST_IDLE = 1;
var ST_IDLE_2 = 2;
// the local-state machine inits + registers at MODULE scope on every eval, the
// proven template idiom.
var __hasInitLocalStateMachine = false;
if (!__hasInitLocalStateMachine) {
	Common.initLocalStateMachine();
	__hasInitLocalStateMachine = true;
}
Common.registerLocalState(ST_ENTRANCE, "entrance");
Common.registerLocalState(ST_FALL, "fall");
Common.registerLocalState(ST_IDLE, "idle");
Common.registerLocalState(ST_IDLE_2, "idle");
var _t_m_delayTimer = self.makeFrameTimer((60) * 2);
var _t_m_waitTimer = self.makeFrameTimer((30 * 3) * 2);

// stage spawn machine constants (stepped from the stage + enemy classes)
var COLUMNS = [293.6, 445.7, 597.8, 950.1, 1102.2, 1254.3];
var LAND_YS = [692.0, 692.0, 692.0, 692.0, 692.0, 692.0];
var SPAWN_Y = -7.1;
var SPAWN_PERIOD = 1200;
var _w_init = self.makeBool(false);
var _w_active = self.makeBool(false);
var _w_col = self.makeInt(0);
// the stage's spawn machine (one full spawn-to-spawn period) + the rehit cadence
var _w_clock = self.makeFrameTimer(SPAWN_PERIOD);
var _w_cool = self.makeFrameTimer(60);
var _w_prev = self.makeInt(-99);
var _w_state_t = self.makeInt(0);
var _sp_deck = self.makeInt(-1);
var _sp_ceil = self.makeInt(-1);

function __createSelfPlatform() {
	if (_sp_deck.get() >= 0) {
		return;
	}
	var n = match.getStructures().length;
	// solid body outline (riders stand on top; walls block) + the CEILING underside
	match.createLineSegmentStructure([-71.5, -156.0, 84.5, -156.0, 84.5, 13.0, -71.5, 13.0, -71.5, -156.0], new StructureStats({ startX: -2000, startY: -3000, leftLedge: false, rightLedge: false }));
	match.createLineSegmentStructure([84.5, 13.0, -71.5, 13.0], new StructureStats({ startX: -2000, startY: -3000, structureType: StructureType.CEILING }));
	_sp_deck.set(n);
	_sp_ceil.set(n + 1);
	// the body never grounds on its own platform (SSF2 self-platform semantics)
	match.getStructures()[_sp_deck.get()].addToBlacklist(self);
	match.getStructures()[_sp_ceil.get()].addToBlacklist(self);
}

function __selfPlatformDisabled(b:Bool) {
	if (_sp_deck.get() < 0) {
		return;
	}
	match.getStructures()[_sp_deck.get()].updateStructureStats({ disabled: b });
	match.getStructures()[_sp_ceil.get()].updateStructureStats({ disabled: b });
}

function __hazardInit() {
	// [SSF2-only: forceAttack] self.forceAttack("entrance");
	Common.toLocalState(ST_ENTRANCE);
	// FrameTimer construction -> the module-scope makeFrameTimer
	// FrameTimer construction -> the module-scope makeFrameTimer
	__createSelfPlatform(); // SSF2 createSelfPlatform: deck + ceiling structures (helpers below)
	__selfPlatformDisabled(true); // SSF2 setFallthrough(true)
	// [needs-port] self.setCamBoxSize(110 + 30, 130 + 30, -15, -15);
	match.getCamera().addTarget(self);
	return;
}


function __hazardUpdate() {
	var _v1 = null;
	_v1 = null;
	// --------- SUBSTATE SYSTEM ----------
	if (Common.inLocalState(ST_ENTRANCE)) {
		_t_m_delayTimer.tick();
		if (_t_m_delayTimer.completed) {
			Common.toLocalState(ST_FALL);
			self.updateGameObjectStats({ gravity: 9.75, terminalVelocity: 19.50 }); // SSF2 gravity 30 @30fps
			__selfPlatformDisabled(false); // SSF2 setFallthrough(false)
		}
	} else if (Common.inLocalState(ST_FALL)) {
		if (self.isOnFloor()) {
			Common.toLocalState(ST_IDLE);
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
	} else if (Common.inLocalState(ST_IDLE)) {
		_t_m_waitTimer.tick();
		if (_t_m_waitTimer.completed) {
			Common.toLocalState(ST_IDLE_2);
			self.unattachFromFloor(); // SSF2 unnattachFromGround
			self.updateGameObjectStats({ gravity: 0.00 }); // SSF2 gravity 0 @30fps
			self.setYVelocity(-3.90); // SSF2 setYSpeed -6 @30fps
			_t_m_waitTimer.reset();
		}
	} else if (Common.inLocalState(ST_IDLE_2)) {
	}
}


function update() {
	// one-time setup on the first update: the stage script positions this object right
	// after createCustomGameObject (initialize() runs inside the create call, so a park
	// set there would be overridden), and make* slots aren't live at module scope.
	if (!_w_init.get()) {
		_w_init.set(true);
		self.setState(PState.ACTIVE);
		self.setX(COLUMNS[0]);
		self.setY(SPAWN_Y);
		// SSF2's first spawn lands half a period in: pre-advance the spawn clock.
		for (i in 0...600) {
			_w_clock.tick();
		}
	}
	// spawn-to-spawn clock (the stage spawn machine runs regardless of the enemy's phase)
	_w_clock.tick();
	if (!_w_active.get()) {
		if (_w_clock.completed) {
			_w_clock.reset();
			_w_col.set(Random.getInt(0, COLUMNS.length - 1));
			self.setX(COLUMNS[_w_col.get()]);
			self.setY(SPAWN_Y);
			self.setYVelocity(0);
			self.updateGameObjectStats({ gravity: 0 });
			_t_m_delayTimer.reset();
			_t_m_waitTimer.reset();
			__hazardInit();
			_w_active.set(true);
		}
		return;
	}
	// frames-in-state clock for the entrance bob ladder
	if (Common.getLocalState() != _w_prev.get()) {
		_w_prev.set(Common.getLocalState());
		_w_state_t.set(0);
	} else {
		_w_state_t.inc();
	}
	_w_cool.tick();
	if (_w_cool.completed) {
		self.reactivateHitboxes();
		_w_cool.reset();
	}
	// entrance bob: the entrance sub-clip's frame scripts (setYSpeed timeline, 30->60fps)
	if (Common.inLocalState(ST_ENTRANCE)) {
		if (_w_state_t.get() == 0) {
			self.setYVelocity(5.20);
		}
		if (_w_state_t.get() == 58) {
			self.setYVelocity(0.00);
		}
		if (_w_state_t.get() == 112) {
			self.setYVelocity(-2.60);
		}
	}
	__hazardUpdate();
	// the self-platform rides the body (SSF2 moves it with the enemy)
	if (_sp_deck.get() >= 0) {
		match.getStructures()[_sp_deck.get()].setX(self.getX());
		match.getStructures()[_sp_deck.get()].setY(self.getY());
		match.getStructures()[_sp_ceil.get()].setX(self.getX());
		match.getStructures()[_sp_ceil.get()].setY(self.getY());
	}
	// SSF2 culls it past the death bounds (surviveDeathBounds=false); recycle for the next spawn
	if (self.getYVelocity() < 0 && self.getY() <= SPAWN_Y) {
		self.setY(SPAWN_Y);
		self.setYVelocity(0);
		match.getCamera().deleteTarget(self);
		// park the self-platform off-world (carrying a rider out = KO, the SSF2 outcome)
		if (_sp_deck.get() >= 0) {
			match.getStructures()[_sp_deck.get()].setX(-2000);
			match.getStructures()[_sp_deck.get()].setY(-3000);
			match.getStructures()[_sp_ceil.get()].setX(-2000);
			match.getStructures()[_sp_ceil.get()].setY(-3000);
		}
		_w_active.set(false);
	}
}