// The source's static API as a stage's code asks it, and its objects' code: the members only a
// stage's scripts answer, put on the API as the script starts (a fighter's script carries none of
// this).

// class PeptideAPI
var StageAPI = {};
StageAPI.install = function() {
	PeptideAPI.getPlayers = function() { return Objects.views(match.getCharacters()); };
	// whether the match has hazards on: the match's own switch
	PeptideAPI.isHazardsOn = function() { var cfg = match.getMatchSettingsConfig(); return cfg == null || cfg.hazards == true; };
	// no final smash and no cutscene of one here
	PeptideAPI.currentActiveFinalSmash = function() { return null; };
	PeptideAPI.isFSCutscenePlaying = function() { return false; };
	// the smash ball is the game's own item: none is here, and the stage cannot ask for one
	PeptideAPI.getSmashBallInstance = function() { return null; };
	PeptideAPI.enableSmashBallSpawn = function(?on) { PeptideRuntime.gap("enableSmashBallSpawn: the game's smash ball"); };
	// what the stage keeps under a name: a surface (a platform) or a region (a collision boundary),
	// asked of the stage's script, which files them
	PeptideAPI.getPlatform = function(n) { return StageAPI.named(AS3.str(n)); };
	// the target placed as a clip: the object the stage put down under that clip's name
	PeptideAPI.getTarget = function(x) {
		if (x == null) return null;
		var o = x.__object;
		if (o == null && x.name != null && x.__auto != true) o = StageAPI.named(AS3.str(x.name));
		return o;
	};
	PeptideAPI.getCollisionBoundary = function(n) { return StageAPI.named(AS3.str(n)); };
	// the enemy a clip belongs to (the one whose clip it is in, asked from inside it)
	PeptideAPI.getEnemy = function(x) { var o = PeptideAPI.objectOf(x); return AS3.is(o, "PeptideEnemy") ? o : null; };
	// the camera's hold on the stage is this engine's
	PeptideAPI.setCamStageFocus = function(?n) { PeptideRuntime.gap("setCamStageFocus: the camera's hold on the stage"); };
	// the source's engine re-sets its camera backgrounds after a stage changes them; the engine here
	// draws and pans them itself
	PeptideAPI.fixBG = function() { PeptideRuntime.gap("fixBG: the camera backgrounds set again"); };
	// the enemies on the stage: every object here running a class that is one
	PeptideAPI.getEnemies = function() {
		var out = [];
		var all = match.getCustomGameObjects();
		if (all != null) for (h in all) { var o = Objects.view(h); if (AS3.is(o, "PeptideEnemy")) out.push(o); }
		return out;
	};
	// the platforms on the stage: every object here running a class that is one
	PeptideAPI.getPlatforms = function() {
		var out = [];
		var all = match.getCustomGameObjects();
		if (all != null) for (h in all) { var o = Objects.view(h); if (AS3.is(o, "PeptidePlatform")) out.push(o); }
		return out;
	};
	// a projectile of a class the package converted, fired by the object given (else the one asking)
	PeptideAPI.spawnProjectile = function(c, ?owner) {
		var o = owner != null ? owner : PeptideRuntime.object;
		return Projectiles.fire(o, Std.isOfType(c, String) ? c : c.name, 0, 0);
	};
	// the match is live once the stage's code runs (it waits for the fighters)
	PeptideAPI.isGameStarted = function() { return true; };
	// the items the match draws from, and how often: the source's are its game's own items, which
	// this engine's match does not carry
	PeptideAPI.getAvailableItemList = function() { PeptideRuntime.gap("getAvailableItemList: the game's own items"); return []; };
	PeptideAPI.getItemFrequency = function() { PeptideRuntime.gap("getItemFrequency: the game's own items"); return 0; };
};
StageAPI.named = function(n) {
	var ex = StageObjects.home();
	return (ex == null || ex.__named == null) ? null : ex.__named(n);
};
