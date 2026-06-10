// API Script for bowserscastlessf2 (converted from SSF2)

var m_hazardsSpawned = false;
function initialize() {
	// animated stage clips play + loop on the timeline
}
function update() {
	if (!m_hazardsSpawned) {
		var chars = match.getCharacters();
		if (chars.length > 0) {
			m_hazardsSpawned = true;
			var owner = null;
			match.createStructure(self.getResource().getContent("bowserscastlessf2platform0"));
			match.createStructure(self.getResource().getContent("bowserscastlessf2platform1"));
			var _hz0 = match.createCustomGameObject(self.getResource().getContent("bowserscastlessf2hazard0"), owner);
			if (_hz0 != null) { _hz0.setX(772.9); _hz0.setY(1057.8); }
			var _hz1 = match.createCustomGameObject(self.getResource().getContent("bowserscastlessf2hazard1"), owner);
			if (_hz1 != null) { _hz1.setX(750.8); _hz1.setY(-27.1); }
		}
	}
}
function onTeardown() {}
function onKill() {}
function onStale() {}
function afterPushState() {}
function afterPopState() {}
function afterFlushStates() {}
