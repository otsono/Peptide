// API Script for sandbag — converted from SSF2
// Frame scripts are embedded in the entity file (FRAME_SCRIPT layers).
// SSF2 API calls are mapped to Fraymakers equivalents where possible.
// Lines marked TODO need manual review.

// ── Instance variables (from SSF2 sandbagExt) ──────────────────────────
var effects;
var clearListener;
var specialEvent;

// start general functions ---

//Runs on object init
function initialize(){
	self.addEventListener(GameObjectEvent.LINK_FRAMES, handleLinkFrames, {persistent:true});
	self.effects = new Array();
	Engine.log("... Sandbag loaded okay, if you were wondering.");
	return;
}

function update(){
}

// Runs when reading inputs (before determining character state, update, framescript, etc.)
function inputUpdateHook(pressedControls:ControlsObject, heldControls:ControlsObject) {
}

// CState-based handling for LINK_FRAMES
function handleLinkFrames(e){
}

function onTeardown() {
}

// --- end general functions

// ── Decompiled from SSF2 XxxExt.as ─────────────────────────────────────────

function addEffectToList(arg0) {
	if (arg0 == null) {
		Engine.log("Tried to add a NULL effect to list!");
		return null;
	} else {
		self.effects.push(arg0);
		return arg0;
	}
}


function bounce(arg0) {
	self.preLand();
	self.removeTimer(self.setSpeed);
	self.removeEventListener(SSF2Event.GROUND_TOUCH, self.bounce);
	self.removeEventListener(SSF2Event.ATTACK_CONNECT, self.bounce);
	if (self.getCharacterStat("jumpSpeedList") != null && self.getJumpSpeed() > self.bounceSpeed) {
	}
}


function clearEffectsOnStateChange(arg0) {
	self.clearListener = arg0;
	self.addEventListener(GameObjectEvent.LINK_FRAMES, self.removeAllEffects);
	return;
}


function damage(arg0) {
	return;
}


function dashCheck() {
	self.setXVelocity(0);
	self.setYVelocity(0);
	self.controls = self.getHeldControls();
	if (self.controls.LEFT) {
		self.dir = "left";
	}
	if (self.controls.RIGHT) {
		self.dir = "right";
	}
	if (self.controls.UP) {
		self.dir = "up";
	}
	if (self.dir == null) {
		return;
	}
	self.removeTimer(self.dashCheck);
	if (self.dir == "up") {
	} else {
		self.addEventListener(SSF2Event.GROUND_TOUCH, self.toLand);
	}
	// [SSF2-only: gotoAndStop] self.gotoAndStop("dash" + self.dir);
}


function dodgeLand(arg0) {
	self.toState(CState.LAND);
	self.playLabel("dodgeland");
	return;
}


function dropItem(arg0) {
	_v2 = null;
	if (self.item) {
		_v2 = self.getCharacterStat("tiltTossMultiplier");
		self.updateCharacterStats({ tiltTossMultiplier: 0.1 });
		// [SSF2-only: tossItem] self.tossItem(270);
		self.updateCharacterStats({ tiltTossMultiplier: self.getCharacterStat("tiltTossMultiplier") });
		self.removeEventListener(GameObjectEvent.LINK_FRAMES, self.dropItem);
	}
	return;
}


function explode(arg0) {
	if (self.item) {
		self.item.removeEventListener(SSF2Event.ATTACK_CONNECT, self.toHelpless);
		self.item.OVERRIDE = true;
		self.item.updateHitboxStats(1, { angle: 75, damage: 17 });
		self.item.explode();
		self.item.updateHitboxStats(1, { angle: 75, damage: 17 });
	}
	return;
}


function flipX(arg0) {
	if (self.isFacingRight()) {
		return arg0;
	}
	return arg0 * -1;
}


function getJumpSpeed() {
	_v1 = self.getCharacterStat("jumpSpeedList").split(",");
	return self.Number(self.getCharacterStat("jumpSpeedList").split(",")[0]);
}


function initShake() {
	self.shake_start_x = x;
	self.shake_start_y = y;
	self.addTimer(1, 2, self.shake);
	return;
}


function jumpToContinue(arg0) {
	self.removeEventListener(SSF2Event.GROUND_TOUCH, self.jumpToContinue);
	self.updateAttackStats({ allowControl: false, cancelWhenAirborne: true });
	self.playLabel("continue");
	return;
}


function moveUp(arg0) {
	self.setYVelocity(self.speed);
	if (self.updateStats) {
		self.updateHitboxStats(1, { baseKnockback: -self.speed * 5 });
		self.updateHitboxStats(2, { baseKnockback: -self.speed * 4 });
	}
	self.speed = self.speed + 2;
	return;
}


function pushEffectBehind(arg0) {
	// [SSF2-only: getMC] SSF2API.getStage().getMidground().swapChildren(self.getMC(), arg0);
	return;
}


function removeAllEffects(arg0) {
	var i = 0;
	while (i < self.effects.length) {
		if (self.effects[i] == null) {
			i = i + 1;
		} else {
			if (self.effects[i].parent == null) {
			} else {
				self.effects[i].parent.removeChild(self.effects[i]);
			}
		}
	}
	self.effects = new Array();
	if ((self.clearListener && self.hasEventListener(GameObjectEvent.LINK_FRAMES, self.removeAllEffects)) || arg0 != null) {
		self.removeEventListener(GameObjectEvent.LINK_FRAMES, self.removeAllEffects);
	}
	return;
}


function removeEvents(arg0) {
	SSF2API.removeEventListener(SSF2Event.GAME_TICK_END, self.updatePalette);
	return;
}


function restoreSpecials(arg0) {
	self.setAttackEnabled(true, "b_down");
	self.setAttackEnabled(true, "b_down_air");
	self.removeEventListener(SSF2Event.GROUND_TOUCH, self.restoreSpecials);
	self.removeEventListener(SSF2Event.CHAR_HURT, self.restoreSpecials);
	self.removeEventListener(SSF2Event.CHAR_GRABBED, self.restoreSpecials);
	self.removeEventListener(SSF2Event.CHAR_LEDGE_GRAB, self.restoreSpecials);
	return;
}


function setLandingLag(arg0) {
	if (arg0) {
		self.removeEventListener(SSF2Event.GROUND_TOUCH, self.toLand);
		self.addEventListener(SSF2Event.GROUND_TOUCH, self.jumpToContinue);
		if (self.isOnFloor()) {
			// [SSF2-only: jumpToContinue] self.jumpToContinue();
		}
		return;
	}
	self.removeEventListener(SSF2Event.GROUND_TOUCH, self.jumpToContinue);
	self.addEventListener(SSF2Event.GROUND_TOUCH, self.toLand);
	if (self.isOnFloor()) {
		self.toState(CState.LAND);
	}
}


function setSpeed(arg0) {
	self.speed = self.speed + 3;
	self.bounceSpeed = self.bounceSpeed + 1.5;
	self.setYVelocity(self.speed);
	return;
}


function shake() {
	self.x.x = self.shake_start_x + SSF2API.safeRandomInteger(-6, 6);
	self.y.y = self.shake_start_y + SSF2API.safeRandomInteger(-6, 6);
	return;
}


function standCountdown(arg0) {
	if (self.getAnimationStatsMetadata(/* TODO: getGlobalVariable */ "standtime") != 0) {
	} else {
		/* ? */.self.updateAnimationStatsMetadata(/* TODO: setGlobalVariable */ "standtime", self.getAnimationStatsMetadata(/* TODO: getGlobalVariable */ "standtime") - 1);
		return;
	}
}


function stopListening() {
	self.clearListener = false;
	self.removeEventListener(GameObjectEvent.LINK_FRAMES, self.removeAllEffects);
	return;
}


function toContinue(arg0) {
	self.charGrabbed = true;
	self.updateAttackStats({ air_ease: 0, cancelWhenAirborne: false });
	// [SSF2-only: unnattachFromGround] self.unnattachFromGround();
	// [SSF2-only: gotoAndStop] self.gotoAndStop("continue");
	self.removeEventListener(SSF2Event.CHAR_GRAB, self.toContinue);
	return;
}


function toEnd(arg0) {
	// [SSF2-only: gotoAndStop] self.gotoAndStop("end");
	self.removeTimer(self.toEnd);
	return;
}


function uncrouch(arg0) {
	if (arg0.data.fromState == 12 && self.getAnimationStatsMetadata(/* TODO: getGlobalVariable */ "crouchdown")) {
	}
}


function updateCharge() {
	if (self.charge != 0) {
	} else {
		// [SSF2-only: attachEffect] self.attachEffect("global_dust_cloud", { scaleX: 0.4, scaleY: 0.4 });
		SSF2API.getCamera().shake(1);
		return;
	}
	if (self.charge != 1) {
	} else {
		// [SSF2-only: attachEffect] self.attachEffect("global_dust_cloud", { scaleX: 0.5, scaleY: 0.5 });
		SSF2API.getCamera().shake(2);
	}
	if (self.charge != 2) {
	} else {
		// [SSF2-only: attachEffect] self.attachEffect("global_dust_cloud", { scaleX: 0.6, scaleY: 0.6 });
		SSF2API.getCamera().shake(5);
	}
}


function updatePalette(arg0) {
	if (!self.didCutin && self.cutin) {
	}
	if (/* ? */) {
		SSF2Utils.replacePalette(self.sandbag, self.costumeData.paletteSwap, 2);
	}
	if (self.costumeData && self.cutin) {
	}
}


