// Character stats for sandbag — converted from SSF2
// SSF2 physics values are scaled to Fraymakers equivalents.
// Review all values before use — units differ between engines.
{
	spriteContent: self.getResource().getContent("sandbag"),

	//GENERIC STATS
	baseScaleX: 1,
	baseScaleY: 1,
	weight: 100,
	gravity: 0.78,
	shortHopSpeed: 7.11 /*TODO: set manually*/,
	jumpSpeed: 12.93,
	doubleJumpSpeeds: [12.93],
	terminalVelocity: 9.96,
	fastFallSpeed: 11.38,
	friction: 0.57 /*TODO*/,
	walkSpeedInitial: 1.0,
	walkSpeedAcceleration: 0.3,
	walkSpeedCap: 6.5,
	dashSpeed: 6.18,
	runSpeedInitial: 4.75,
	runSpeedAcceleration: 0.55,
	runSpeedCap: 7.5,
	groundSpeedAcceleration: 0.3,
	groundSpeedCap: 11,
	aerialSpeedAcceleration: 0.45,
	aerialSpeedCap: 3.5,
	aerialFriction: 0.2,

	//ENVIRONMENTAL COLLISION BODY (ECB) STATS
	floorHeadPosition: 86 /*TODO*/,
	floorHipWidth: 29 /*TODO*/,
	floorHipXOffset: 0,
	floorHipYOffset: 0,
	floorFootPosition: 0,
	aerialHeadPosition: 86 /*TODO*/,
	aerialHipWidth: 29 /*TODO*/,
	aerialHipXOffset: 0,
	aerialHipYOffset: 0,
	aerialFootPosition: 25 /*TODO*/,

	//CAMERA BOX STATS
	cameraBoxOffsetX: 25,
	cameraBoxOffsetY: 75,
	cameraBoxWidth: 200,
	cameraBoxHeight: 250,

	//ROLL AND LEDGE JUMP STATS
	techRollSpeed: 18,
	techRollSpeedStartFrame: 7,
	techRollSpeedLength: 1,
	dodgeRollSpeed: 13,
	dodgeRollSpeedStartFrame: 3,
	dodgeRollSpeedLength: 1,
	getupRollSpeed: 15.5,
	getupRollSpeedStartFrame: 2,
	getupRollSpeedLength: 1,
	ledgeRollSpeed: 14,
	ledgeRollSpeedStartFrame: 1,
	ledgeRollSpeedLength: 1,
	ledgeJumpXSpeed: 2.5,
	ledgeJumpYSpeed: -10,

	//AIRDASH STATS
	airdashInitialSpeed: 11,
	airdashSpeedCap: 12.5,
	airdashAccelMultiplier: 0.4,
	airdashCancelSpeedConservation: 0.9,

	//SHIELD STATS
	shieldCrossupThreshold: 16,
	shieldFrontNineSliceContent: "global::vfx.vfx_shield_front",
	shieldFrontXOffset: 10.5,
	shieldFrontYOffset: 4,
	shieldFrontWidth: 53,
	shieldFrontHeight: 93,
	shieldBackNineSliceContent: "global::vfx.vfx_shield_back",
	shieldBackXOffset: 12.5,
	shieldBackYOffset: 4,
	shieldBackWidth: 49,
	shieldBackHeight: 93,

	//VOICE STATS
	attackVoiceIds: [],
	hurtLightVoiceIds: [],
	hurtMediumVoiceIds: [],
	hurtHeavyVoiceIds: [],
	koVoiceIds: [],
	attackVoiceSilenceRate: 0.5,
	hurtLightSilenceRate: 1,
	hurtMediumSilenceRate: 0.5,
	hurtHeavySilenceRate: 0,
	koVoiceSilenceRate: 0,
}
