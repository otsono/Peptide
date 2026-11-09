// what a field that only ever holds a new one of a class is written as: the class's state and calls
// as the script's own, each named after the field (`soundLoop_elapsed`, `soundLoop_tick()`), so a
// rollback puts them back. a read of its `x` is `FIELD_get_x()`, a write `FIELD_set_x(v)`, a call
// of its `m` is `FIELD_m(...)`, and a new one is `FIELD_new(...)`. only what the code uses is written.
//
// `//:` lines are notes for whoever edits this, left out of what is written; `//: bool x` says
// `FIELD_x` answers true or false.

// ---- FrameTimer
//: bool get_completed
// FIELD: a countdown in frames, ticked on, complete once it has run its length
var FIELD_length = self.makeInt(0);
var FIELD_elapsed = self.makeInt(0);
var FIELD_data = self.makeObject(null);
function FIELD_new(?frames) { FIELD_length.set(frames == null ? 0 : Std.int(frames)); FIELD_elapsed.set(0); }
function FIELD_tick(?n) { FIELD_set_elapsedFrames(FIELD_elapsed.get() + (n == null ? 1 : n)); }
function FIELD_reset() { FIELD_elapsed.set(0); }
function FIELD_finish() { FIELD_elapsed.set(FIELD_length.get()); }
function FIELD_get_completed() { return FIELD_elapsed.get() >= FIELD_length.get(); }
function FIELD_get_elapsedFrames() { return FIELD_elapsed.get(); }
function FIELD_set_elapsedFrames(v) { FIELD_elapsed.set(Std.int(Math.max(0, Math.min(v, FIELD_length.get())))); }
function FIELD_get_duration() { return FIELD_length.get(); }
function FIELD_set_duration(v) { FIELD_length.set(Std.int(Math.max(0, v))); FIELD_elapsed.set(Std.int(Math.min(FIELD_elapsed.get(), FIELD_length.get()))); }
function FIELD_get_data() { return FIELD_data.get(); }
function FIELD_set_data(v) { FIELD_data.set(v); }
