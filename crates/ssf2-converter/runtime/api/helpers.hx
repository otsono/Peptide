// Small helpers content builds on: a frame countdown, a set of held buttons as bits, a beacon, and
// the final-smash meter class some packages reference.

// a countdown in frames: ticked on, it is complete once it has run its length
// class PeptideFrameTimer extends Object
var PeptideFrameTimer = { name: "PeptideFrameTimer" };
PeptideFrameTimer.build = function(?me) {
	me.__is = ["PeptideFrameTimer"];
	me.__length = 0;
	me.__ran = 0;
	me.__data = null;
	me.get_data = function() { return me.__data; };
	me.set_data = function(?v) { me.__data = v; return v; };
	me.get_completed = function() { return me.__ran >= me.__length; };
	me.get_duration = function() { return me.__length; };
	me.set_duration = function(?v) { me.__length = Math.max(0, AS3.num(v)); me.__ran = Math.min(me.__ran, me.__length); return v; };
	me.get_elapsedFrames = function() { return me.__ran; };
	me.set_elapsedFrames = function(?v) { me.__ran = Math.max(0, Math.min(AS3.num(v), me.__length)); return v; };
	// ticked by code the source runs every frame, it moves half a frame each of this engine's
	me.tick = function(?n) { me.set_elapsedFrames(me.__ran + (n == null ? 1 : AS3.num(n))); };
	me.__copy = function() { var t = PeptideFrameTimer.create(me.__length); t.__ran = me.__ran; t.__data = me.__data; return t; };
	me.finish = function() { me.__ran = me.__length; };
	me.reset = function() { me.__ran = 0; };
};
PeptideFrameTimer.init = function(?me, ?frames) { me.__length = frames == null ? 0 : AS3.num(frames); };
PeptideFrameTimer.create = function(?frames) { var me = {}; PeptideFrameTimer.build(me); PeptideFrameTimer.init(me, frames); return me; };

// held buttons as one number, a bit each, read and written by button
// class PeptideControlBits extends Object
var PeptideControlBits = { name: "PeptideControlBits" };
PeptideControlBits.TAP_JUMP = 1; PeptideControlBits.SHIELD = 2; PeptideControlBits.TAUNT = 4; PeptideControlBits.START = 8; PeptideControlBits.GRAB = 16;
PeptideControlBits.BUTTON2 = 32; PeptideControlBits.BUTTON1 = 64; PeptideControlBits.JUMP = 128; PeptideControlBits.RIGHT = 256; PeptideControlBits.LEFT = 512;
PeptideControlBits.DOWN = 1024; PeptideControlBits.UP = 2048; PeptideControlBits.DT_DASH = 4096; PeptideControlBits.AUTO_DASH = 8192; PeptideControlBits.DASH = 16384;
PeptideControlBits.C_RIGHT = 32768; PeptideControlBits.C_LEFT = 65536; PeptideControlBits.C_DOWN = 131072; PeptideControlBits.C_UP = 262144;
PeptideControlBits.JUMP2 = 524288; PeptideControlBits.SHIELD2 = 1048576; PeptideControlBits.JUMP3 = 2097152;
PeptideControlBits.build = function(?me) {
	me.__is = ["PeptideControlBits"];
	me.bits = 0;
	me.__copy = function() { var c = PeptideControlBits.create(); c.bits = me.bits; return c; };
	var bit = function(?b) { return function() { return (me.bits & b) != 0; }; };
	var put = function(?b) { return function(?on) { me.bits = AS3.truthy(on) ? (me.bits | b) : (me.bits & ~b); return on; }; };
	me.get_TAP_JUMP = bit(1); me.set_TAP_JUMP = put(1); me.get_SHIELD = bit(2); me.set_SHIELD = put(2);
	me.get_TAUNT = bit(4); me.set_TAUNT = put(4); me.get_START = bit(8); me.set_START = put(8);
	me.get_GRAB = bit(16); me.set_GRAB = put(16); me.get_BUTTON2 = bit(32); me.set_BUTTON2 = put(32);
	me.get_BUTTON1 = bit(64); me.set_BUTTON1 = put(64); me.get_JUMP = bit(128); me.set_JUMP = put(128);
	me.get_RIGHT = bit(256); me.set_RIGHT = put(256); me.get_LEFT = bit(512); me.set_LEFT = put(512);
	me.get_DOWN = bit(1024); me.set_DOWN = put(1024); me.get_UP = bit(2048); me.set_UP = put(2048);
	me.get_DT_DASH = bit(4096); me.set_DT_DASH = put(4096); me.get_AUTO_DASH = bit(8192); me.set_AUTO_DASH = put(8192);
	me.get_DASH = bit(16384); me.set_DASH = put(16384); me.get_C_RIGHT = bit(32768); me.set_C_RIGHT = put(32768);
	me.get_C_LEFT = bit(65536); me.set_C_LEFT = put(65536); me.get_C_DOWN = bit(131072); me.set_C_DOWN = put(131072);
	me.get_C_UP = bit(262144); me.set_C_UP = put(262144); me.get_JUMP2 = bit(524288); me.set_JUMP2 = put(524288);
	me.get_SHIELD2 = bit(1048576); me.set_SHIELD2 = put(1048576); me.get_JUMP3 = bit(2097152); me.set_JUMP3 = put(2097152);
};
PeptideControlBits.init = function(?me, ?bits) { me.bits = bits == null ? 0 : Std.int(AS3.num(bits)); };
PeptideControlBits.create = function(?bits) { var me = {}; PeptideControlBits.build(me); PeptideControlBits.init(me, bits); return me; };

// a placed marker object: a body with nothing of its own
// class PeptideBeacon extends PeptideGameObject
var PeptideBeacon = { name: "PeptideBeacon" };
PeptideBeacon.build = function(?me) {
	PeptideGameObject.build(me);
	me.__is.push("PeptideBeacon");
	me.__type = "SSF2Beacon";
};
PeptideBeacon.init = function(?me, ?api) {};
PeptideBeacon.create = function(?api) { var me = {}; PeptideBeacon.build(me); PeptideBeacon.init(me, api); return me; };

// a meter under the fighter's damage counter that shows how much of something is left: the
// package's own `fs_bar` clip, its `bar` scaled by what is left of the most
// class PeptideFinalSmashBar extends Object
var PeptideFinalSmashBar = { name: "PeptideFinalSmashBar" };
PeptideFinalSmashBar.build = function(?me) {
	me.__is = ["PeptideFinalSmashBar"];
	me.__left = 0;
	me.__most = 0;
	me.__clip = null;
};
PeptideFinalSmashBar.addToDamageMeter = function(me, ?box) {
	if (box == null) return;
	me.__clip = box.addChild(PeptideAPI.clipByLinkage("fs_bar"));
	if (me.__clip != null) { me.__clip.x = 1.35; me.__clip.y = 41.4; }
};
PeptideFinalSmashBar.removeFromDamageMeter = function(me, ?box) { if (box != null && me.__clip != null) box.removeChild(me.__clip); };
PeptideFinalSmashBar.updateBar = function(me, ?v) {
	me.__left = AS3.num(v);
	if (me.__clip != null && me.__clip.bar != null && me.__most != 0) me.__clip.bar.scaleX = me.__left / me.__most;
};
PeptideFinalSmashBar.init = function(?me, ?most) { me.__most = most == null ? 0 : AS3.num(most); me.__left = me.__most; };
PeptideFinalSmashBar.create = function(?most) { var me = {}; PeptideFinalSmashBar.build(me); PeptideFinalSmashBar.init(me, most); return me; };

// the clip a package's own root class extends: a store of named values, and the API handed to it as
// it starts (its lock to the source's own domains has nothing to lock here)
// class PeptideAsset extends MovieClip
var PeptideAsset = { name: "PeptideAsset" };
PeptideAsset.build = function(me) {
	MovieClip.build(me);
	me.__is.push("PeptideAsset");
	me.__props = PeptideRuntime.cell(null);
	me.register = function(?k, ?v) {
		var d = AS3.copyOf(me.__props.get());
		if (d == null) d = Dictionary.create();
		AS3.put(d, k, v);
		me.__props.set(d);
	};
	me.getProp = function(?k) { var d = me.__props.get(); return d == null ? null : AS3.at(d, k); };
	me.initAPI = function(?src) { return PeptideAPI.init(src); };
	me.deinitAPI = function() { PeptideAPI.deinit(); };
};
PeptideAsset.init = function(me) { MovieClip.init(me); };
PeptideAsset.create = function() { var me = {}; PeptideAsset.build(me); PeptideAsset.init(me); return me; };
