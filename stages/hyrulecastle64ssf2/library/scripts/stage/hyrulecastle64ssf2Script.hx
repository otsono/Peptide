// API Script for hyrulecastle64ssf2 (converted from SSF2)

var m_hazardsSpawned = false;
function initialize() {
	self.pause();
}
function update() {
	if (!m_hazardsSpawned) {
		var chars = match.getCharacters();
		if (chars.length > 0) {
			m_hazardsSpawned = true;
			var owner = null;
			var _hz0 = match.createCustomGameObject(self.getResource().getContent("hyrulecastle64ssf2hazard0"), owner);
			if (_hz0 != null) { _hz0.setX(122.0); _hz0.setY(990.1); }
		}
	}
}
function onTeardown() {}
function onKill() {}
function onStale() {}
function afterPushState() {}
function afterPopState() {}
function afterFlushStates() {}
