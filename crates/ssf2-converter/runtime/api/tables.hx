// The constant tables content reads its engine's values from: event names, state numbers, item
// frequencies. The values are the ones content compares against.

// the events an object raises, by the name a listener is added under
// class PeptideEvent extends Object
var PeptideEvent = { name: "PeptideEvent" };
PeptideEvent.GAME_TICK_START = "gameTickStart";
PeptideEvent.GAME_TICK_END = "gameTickEnd";
PeptideEvent.GAME_ITEM_CREATED = "gameItemCreated";
PeptideEvent.GAME_ITEM_DESTROYED = "gameItemDestroyed";
PeptideEvent.GAME_PAUSED = "gamePaused";
PeptideEvent.GAME_UNPAUSED = "gameUnpaused";
PeptideEvent.GAME_ENDED = "gameEnded";
PeptideEvent.STATE_CHANGE = "stateChange";
PeptideEvent.GROUND_TOUCH = "groundTouch";
PeptideEvent.GROUND_LEAVE = "groundLeave";
PeptideEvent.HIT_WALL = "hitWall";
PeptideEvent.HOMING_TARGET = "homingTarget";
PeptideEvent.ATTACK_HIT = "attackHit";
PeptideEvent.ATTACK_CONNECT = "attackConnect";
PeptideEvent.ATTACK_HIT_SHIELD = "attackHitShield";
PeptideEvent.ATTACK_HIT_POWER_SHIELD = "attackHitPowerShield";
PeptideEvent.ATTACK_CONNECT_SHIELD = "attackConnectShield";
PeptideEvent.ATTACK_ENABLED = "attackEnabled";
PeptideEvent.ATTACK_CLANK = "attackClank";
PeptideEvent.ATTACK_COLLIDE = "attackCollide";
PeptideEvent.CHAR_ATTACK_COMPLETE = "charAttackComplete";
PeptideEvent.CHAR_ATTACK_CHANGED = "charAttackChanged";
PeptideEvent.CHAR_ROCKET_COMPLETE = "rocketComplete";
PeptideEvent.CHAR_ATTACK_CANCELLED = "charAttackCancelled";
PeptideEvent.CHAR_COUNTER = "charCounter";
PeptideEvent.CHAR_ABSORB = "charAbsorb";
PeptideEvent.CHAR_GRABBED = "charGrabbed";
PeptideEvent.CHAR_GRAB = "charGrab";
PeptideEvent.CHAR_LEDGE_GRAB = "charLedgeGrab";
PeptideEvent.CHAR_HURT = "charHurt";
PeptideEvent.CHAR_DAMAGE = "charDamage";
PeptideEvent.DAMAGE_CHANGED = "damageChanged";
PeptideEvent.CHAR_WIND = "charWind";
PeptideEvent.CHAR_SELF_DESTRUCT = "charSelfDestruct";
PeptideEvent.CHAR_TRANSFORM = "charTransform";
PeptideEvent.CHAR_KO_DEATH = "charKODeath";
PeptideEvent.CHAR_KO_POINT = "charKOPoint";
PeptideEvent.CHAR_SHIELD_HIT = "charShieldHit";
PeptideEvent.CHAR_POWER_SHIELD_HIT = "charPowerShieldHit";
PeptideEvent.CHAR_LAND = "charLand";
PeptideEvent.CHAR_SIZE_CHANGE = "charSizeChange";
PeptideEvent.CHAR_METAL_CHANGE = "charMetalChange";
PeptideEvent.CHAR_DESTROYED = "charDestroyed";
PeptideEvent.PROJ_X_DECAY_COMPLETE = "projXDecayComplete";
PeptideEvent.PROJ_DESTROYED = "projDestroyed";
PeptideEvent.PROJ_LATCHED = "projLatched";
PeptideEvent.PROJ_COLLIDE = "projCollide";
PeptideEvent.PROJ_COLLIDE_OWNER = "projCollideOwner";
PeptideEvent.PROJ_HURT = "projHurt";
PeptideEvent.ITEM_HURT = "itemHurt";
PeptideEvent.ITEM_WIND = "itemWind";
PeptideEvent.ITEM_TOUCH = "itemTouch";
PeptideEvent.ITEM_DESTROYED = "itemDestroyed";
PeptideEvent.ITEM_PICKUP = "itemPickup";
PeptideEvent.ITEM_TOSSED = "itemTossed";
PeptideEvent.ENEMY_DESTROYED = "enemyDestroyed";
PeptideEvent.ENEMY_HURT = "enemyHurt";
PeptideEvent.ENEMY_WIND = "enemyWind";
PeptideEvent.TARGET_DESTROYED = "targetDestroyed";
PeptideEvent.TARGET_BROKEN = "targetBroken";
PeptideEvent.REVERSE = "reverse";
PeptideEvent.REVERSE_HIT = "reverseHit";

// a fighter's state, as content numbers it
// class PeptideCState extends Object
var PeptideCState = { name: "PeptideCState" };
PeptideCState.IDLE = 0;
PeptideCState.ENTRANCE = 1;
PeptideCState.REVIVAL = 2;
PeptideCState.WALK = 3;
PeptideCState.RUN = 4;
PeptideCState.JUMP_CHAMBER = 5;
PeptideCState.JUMP_RISING = 6;
PeptideCState.JUMP_FALLING = 7;
PeptideCState.LAND = 8;
PeptideCState.ATTACKING = 9;
PeptideCState.LEDGE_ROLL = 10;
PeptideCState.ITEM_TOSS = 11;
PeptideCState.CROUCH = 12;
PeptideCState.GRABBING = 13;
PeptideCState.INJURED = 14;
PeptideCState.FLYING = 15;
PeptideCState.DODGE_ROLL = 16;
PeptideCState.AIR_DODGE = 17;
PeptideCState.SIDESTEP_DODGE = 18;
PeptideCState.LEDGE_HANG = 19;
PeptideCState.LEDGE_CLIMB = 20;
PeptideCState.ROLL = 21;
PeptideCState.SHIELDING = 22;
PeptideCState.STUNNED = 23;
PeptideCState.DIZZY = 24;
PeptideCState.TUMBLE_FALL = 25;
PeptideCState.CRASH_LAND = 26;
PeptideCState.CRASH_GETUP = 27;
PeptideCState.CARRY = 28;
PeptideCState.SWIMMING = 29;
PeptideCState.LADDER = 30;
PeptideCState.EDGE_LEAN = 31;
PeptideCState.WALL_CLING = 32;
PeptideCState.FROZEN = 33;
PeptideCState.TAUNT = 34;
PeptideCState.KIRBY_STAR = 35;
PeptideCState.CAUGHT = 36;
PeptideCState.STAR_KO = 37;
PeptideCState.SCREEN_KO = 38;
PeptideCState.PITFALL = 39;
PeptideCState.SLEEP = 40;
PeptideCState.EGG = 41;
PeptideCState.DEAD = 42;
PeptideCState.GLIDING = 43;
PeptideCState.SKID = 44;
PeptideCState.HOVER = 45;
PeptideCState.JUMP_MIDAIR_RISING = 46;
PeptideCState.DISABLED = 47;
PeptideCState.DASH = 48;
PeptideCState.TURN = 49;
PeptideCState.TECH_GROUND = 50;
PeptideCState.TECH_ROLL = 51;
PeptideCState.SHIELD_DROP = 52;
PeptideCState.BARREL = 53;
PeptideCState.DASH_INIT = 54;
PeptideCState.HEAVY_LAND = 55;
PeptideCState.STAMINA_KO = 56;
PeptideCState.ITEM_PICKUP = 57;
PeptideCState.LOCKED = 58;

// a projectile's state
// class PeptidePState extends Object
var PeptidePState = { name: "PeptidePState" };
PeptidePState.IDLE = 0;
PeptidePState.DEAD = 1;

// an item's state
// class PeptideIState extends Object
var PeptideIState = { name: "PeptideIState" };
PeptideIState.IDLE = 0;
PeptideIState.HOLD = 1;
PeptideIState.TOSS = 2;
PeptideIState.DEAD = 3;

// an enemy's state
// class PeptideEState extends Object
var PeptideEState = { name: "PeptideEState" };
PeptideEState.IDLE = 0;
PeptideEState.DEAD = 1;

// a target's state
// class PeptideTState extends Object
var PeptideTState = { name: "PeptideTState" };
PeptideTState.IDLE = 0;
PeptideTState.BROKEN = 1;
PeptideTState.DEAD = 2;

// how often items appear
// class PeptideItemSettings extends Object
var PeptideItemSettings = { name: "PeptideItemSettings" };
PeptideItemSettings.FREQUENCY_OFF = 0;
PeptideItemSettings.FREQUENCY_VERY_LOW = 1;
PeptideItemSettings.FREQUENCY_LOW = 2;
PeptideItemSettings.FREQUENCY_MEDIUM = 3;
PeptideItemSettings.FREQUENCY_HIGH = 4;
PeptideItemSettings.FREQUENCY_VERY_HIGH = 5;
PeptideItemSettings.FREQUENCY_SUPER_HIGH = 6;
PeptideItemSettings.FREQUENCY_ULTRA_HIGH = 7;
PeptideItemSettings.FREQUENCY_MAX = 8;
