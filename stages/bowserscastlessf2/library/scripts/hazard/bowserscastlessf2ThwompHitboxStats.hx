// HitboxStats for the stage hazard — 1:1 from the SSF2 getAttackStats disasm
// (power -> baseKnockback, kbConstant -> knockbackGrowth, direction -> angle).
{
	entrance: {},
	fall: {
		hitbox0: { damage: 30, angle: 135, baseKnockback: 125, knockbackGrowth: 12, hitstop: 6, hitstun: 24, reversibleAngle: false, directionalInfluence: true, reflectable: false },
		hitbox1: { damage: 30, angle: 45, baseKnockback: 125, knockbackGrowth: 12, hitstop: 6, hitstun: 24, reversibleAngle: false, directionalInfluence: true, reflectable: false }
	},
	idle: {}
}