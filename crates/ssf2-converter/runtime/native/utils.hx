// The API's helpers (`SSF2Utils`) as plain functions: a native object's code that calls one gets
// its block (compile/native.rs), under the API's own name.

// ---- forceBase360
// an angle brought into 0 up to 360
function forceBase360(a) {
	while (a < 0) a += 360;
	while (a >= 360) a -= 360;
	return a;
}

// ---- calculateDifferenceBetweenAngles
// how far one angle is from another, the short way round (-180 up to 180)
function calculateDifferenceBetweenAngles(a, b) {
	var d = b - a;
	while (d < -180) d += 360;
	while (d > 180) d -= 360;
	return d;
}

// ---- getAngleBetween
// the angle from one point to another, up from the right, in 0 up to 360
function getAngleBetween(a, b) {
	var d = Math.atan2(-(b.y - a.y), b.x - a.x) * 180 / Math.PI;
	while (d < 0) d += 360;
	while (d >= 360) d -= 360;
	return d;
}

// ---- decel
// a speed eased toward rest: a non-negative rate scales it, a negative one takes that much off its
// size; it never crosses zero, and under half a pixel it is at rest
function decel(v, d) {
	var next = d >= 0 ? v * d : (v > 0 ? Math.max(0, v + d) : Math.min(0, v - d));
	return Math.abs(next) < 0.5 ? 0 : next;
}

// ---- getVelocityVector
// a speed along an angle, as its x and y
function getVelocityVector(speed, angle) {
	return { x: speed * Math.cos(angle * Math.PI / 180), y: speed * Math.sin(angle * Math.PI / 180) };
}
