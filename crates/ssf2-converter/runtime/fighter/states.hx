// The source's fighter states, on this engine's. The source numbers its states (its CState class,
// which every package ships); this engine names its own. Each source state is a test of which of
// this engine's states it covers, and the one it becomes when the source sets it.

var States = {};

// whether this engine's character is in the source's state `n`
States.is = function(fm, n) {
	// struck this frame: hurt, as the source has it while it tells of the hit (Events.handler)
	if (fm.exports != null && fm.exports.__struck == true) return n == 14;
	switch (n) {
		case 0: return fm.inState(CState.STAND) || fm.inState(CState.STAND_TURN);                     // IDLE
		case 1: return fm.inState(CState.INTRO);                                                        // ENTRANCE
		case 2: return fm.inState(CState.REVIVAL);                                                      // REVIVAL
		case 3: return fm.inState(CState.WALK_IN) || fm.inState(CState.WALK_LOOP) || fm.inState(CState.WALK_OUT); // WALK
		case 4: return fm.inState(CState.RUN) || fm.inState(CState.RUN_TURN);                           // RUN
		case 5: return fm.inState(CState.JUMP_SQUAT);                                                   // JUMP_CHAMBER
		case 6: return fm.inState(CState.JUMP_IN) || fm.inState(CState.JUMP_LOOP);                      // JUMP_RISING
		case 7: return fm.inState(CState.FALL) || fm.inState(CState.JUMP_OUT);                          // JUMP_FALLING
		case 8: return fm.inState(CState.LAND);                                                         // LAND
		case 9: return fm.inStateGroup(CStateGroup.ATTACK);                                             // ATTACKING
		case 10: return fm.inState(CState.LEDGE_ROLL);                                                  // LEDGE_ROLL
		case 12: return fm.inState(CState.CROUCH_IN) || fm.inState(CState.CROUCH_LOOP) || fm.inState(CState.CROUCH_OUT); // CROUCH
		case 13: return fm.inState(CState.GRAB) || fm.inState(CState.GRAB_HOLD) || fm.inState(CState.GRAB_PUMMEL)
			|| fm.inState(CState.THROW_UP) || fm.inState(CState.THROW_DOWN) || fm.inState(CState.THROW_FORWARD) || fm.inState(CState.THROW_BACK); // GRABBING
		case 14: return fm.inState(CState.HURT_LIGHT) || fm.inState(CState.HURT_MEDIUM) || fm.inState(CState.HURT_HEAVY); // INJURED
		case 15: return fm.inState(CState.HURT_HEAVY) || fm.inState(CState.TUMBLE);                     // FLYING
		case 16: return fm.inState(CState.ROLL);                                                        // DODGE_ROLL
		case 18: return fm.inState(CState.SPOT_DODGE);                                                  // SIDESTEP_DODGE
		case 19: return fm.inState(CState.LEDGE_IN) || fm.inState(CState.LEDGE_LOOP) || fm.inState(CState.LEDGE_HANG); // LEDGE_HANG
		case 20: return fm.inState(CState.LEDGE_CLIMB);                                                 // LEDGE_CLIMB
		case 21: return fm.inState(CState.ROLL) || fm.inState(CState.LEDGE_ROLL);                       // ROLL
		case 22: return fm.inState(CState.SHIELD_IN) || fm.inState(CState.SHIELD_LOOP) || fm.inState(CState.SHIELD_HURT) || fm.inState(CState.SHIELD_OUT); // SHIELDING
		case 25: return fm.inState(CState.TUMBLE);                                                      // TUMBLE_FALL
		case 26: return fm.inState(CState.CRASH_BOUNCE) || fm.inState(CState.CRASH_LOOP);               // CRASH_LAND
		case 27: return fm.inState(CState.CRASH_GET_UP);                                                // CRASH_GETUP
		case 36: return fm.inState(CState.GRABBED_HOLD);                                                // CAUGHT
		case 37: return fm.inState(CState.STAR_KO);                                                     // STAR_KO
		case 38: return fm.inState(CState.SCREEN_KO);                                                   // SCREEN_KO
		case 42: return fm.inState(CState.KO);                                                          // DEAD
		case 44: return fm.inState(CState.SKID);                                                        // SKID
		case 46: return fm.inState(CState.JUMP_MIDAIR);                                                 // JUMP_MIDAIR_RISING
		case 47: return fm.inState(CState.DISABLED);                                                    // DISABLED
		case 48: return fm.inState(CState.DASH);                                                        // DASH
		case 49: return fm.inState(CState.STAND_TURN) || fm.inState(CState.RUN_TURN);                   // TURN
		case 50: return fm.inState(CState.TECH);                                                        // TECH_GROUND
		case 51: return fm.inState(CState.TECH_ROLL);                                                   // TECH_ROLL
		case 55: return fm.inState(CState.LAND);                                                        // HEAVY_LAND
		case 56: return fm.inState(CState.STAMINA_KO);                                                  // STAMINA_KO
		default: return false;
	}
};

// the state this engine's character goes to when the source sets state `n` (null: none to go to)
States.enter = function(fm, n) { var s = States.to(n); if (s != null) fm.toState(s); };
States.to = function(n) {
	switch (n) {
		case 0: return CState.STAND;
		case 3: return CState.WALK_IN;
		case 4: return CState.RUN;
		case 5: return CState.JUMP_SQUAT;
		case 6: return CState.JUMP_IN;
		case 7: return CState.FALL;
		case 8: return CState.LAND;
		case 12: return CState.CROUCH_IN;
		case 14: return CState.HURT_LIGHT;
		case 15: return CState.HURT_HEAVY;
		case 16: return CState.ROLL;
		case 18: return CState.SPOT_DODGE;
		case 19: return CState.LEDGE_HANG;
		case 20: return CState.LEDGE_CLIMB;
		case 22: return CState.SHIELD_IN;
		case 25: return CState.TUMBLE;
		case 26: return CState.CRASH_BOUNCE;
		case 27: return CState.CRASH_GET_UP;
		case 44: return CState.SKID;
		case 46: return CState.JUMP_MIDAIR;
		case 48: return CState.DASH;
		case 55: return CState.LAND;
		default: return null;
	}
};

// the source's number for the state this engine's character is in (the first that covers it)
States.of = function(fm) {
	// a target answers its own state (it is no fighter)
	if (fm != null && fm.exports != null && fm.exports.__tstate != null) return fm.exports.__tstate;
	var n = 0;
	while (n < 64) { if (States.is(fm, n)) return n; n++; }
	return -1;
};
