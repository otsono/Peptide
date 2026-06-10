// Thwomp reconstructed from its SSF2 update()/initialize() via the character decompiler,
// then made FM-CGO-runnable (field-state -> self.makeInt, FrameTimer -> counter,
// unmapped calls neutralized so it can't throw).

var _s_m_action = self.makeInt(0);
var _t_m_delayTimer = self.makeInt(0);
var _t_m_waitTimer = self.makeInt(0);

function initialize() {
	// [SSF2-only: forceAttack] self.forceAttack("entrance");
	_s_m_action.set(-1);
	// timer init -> persistent counter (below)
	// timer init -> persistent counter (below)
	// [SSF2-only: createSelfPlatform] self.m_selfPlatform = self.createSelfPlatform(-55, -120, 120, 130);
	// [needs-port] self.m_selfPlatform.setFallthrough(true);
	// [needs-port] self.setCamBoxSize(110 + 30, 130 + 30, -15, -15);
	// [needs-port] match.getCamera().addTarget(self);
	return;
}


function update() {
	var _v1 = null;
	_v1 = null;
	if (_s_m_action.get() != -1) {
		if (_s_m_action.get() != 0) {
			if (_s_m_action.get() != 1) {
				if (_s_m_action.get() != 2) {
				}
				return;
			} else {
				_t_m_waitTimer.set(_t_m_waitTimer.get() + 1);
				if ((_t_m_waitTimer.get() >= 30 * 3)) {
					_s_m_action.set(2);
					// [SSF2-only: unnattachFromGround] self.unnattachFromGround();
					self.updateGameObjectStats({ gravity: 0 });
					self.setYVelocity(-6);
					_t_m_waitTimer.set(0);
				}
			}
		} else {
			if (false) {
			}
			_s_m_action.set(1);
			// [SSF2-only: forceAttack] self.forceAttack("idle");
			// [needs-port] AudioClip.play("thwomp_land");
			// [needs-port] AudioClip.play("thwomp_vfx");
			// [needs-port] match.createVfx(new VfxStats({ spriteContent: "global::vfx.vfx", animation: GlobalVfx.DUST_POOF, scaleX: 2, scaleY: 2 }), self);
			match.getCamera().shake(13);
			// [SSF2-only: cast] _v1 = SSF2Utils.cast(self.getCurrentPlatform(), BowsersCastlePlatform);
			// [SSF2-only: cast] if (SSF2Utils.cast(self.getCurrentPlatform(), BowsersCastlePlatform)) {
				// [SSF2-dead] match.shakeCamera(13);
				// [SSF2-dead] _v1.sink();
			// [SSF2-dead] }
		}
	} else {
		_t_m_delayTimer.set(_t_m_delayTimer.get() + 1);
		if ((_t_m_delayTimer.get() >= 60)) {
			_s_m_action.set(0);
			self.updateGameObjectStats({ gravity: 30 });
			// [needs-port] self.m_selfPlatform.setFallthrough(false);
		}
	}
}

