// Parses every .hx file in a directory and the directories under it as script code, without typing
// it, and prints each file that does not parse with the reason. What the converter emits has to parse before anything
// else about it matters; this checks that with the Haxe toolchain alone, no engine.
//
//   haxe -cp tools/hxcheck --macro "HxCheck.run('<dir>')"
import haxe.macro.Context;

class HxCheck {
	public static function run(dir:String) {
		var bad = 0;
		var files = [];
		function walk(d:String, rel:String) {
			for (f in sys.FileSystem.readDirectory(d)) {
				var p = d + "/" + f;
				if (sys.FileSystem.isDirectory(p)) walk(p, rel + f + "/");
				else if (StringTools.endsWith(f, ".hx")) files.push(rel + f);
			}
		}
		walk(dir, "");
		files.sort(Reflect.compare);
		for (file in files) {
			var code = sys.io.File.getContent(dir + "/" + file);
			// the engine's script parser takes a keyword as an object's key (`break: {...}`, an
			// animation named so); Haxe's does not, so such a key is quoted before parsing
			code = ~/([{,]\s*)(break|continue|switch|function|return|var|if|else|while|for|do|new|try|catch|throw|in|null|true|false|this|class|static|import|package|extends|implements|public|private|inline|macro|override|dynamic|abstract|cast|typedef|enum|untyped|using|interface|operator|overload|final|extern)(\s*:)/g.replace(code, '$1"$2"$3');
			try {
				Context.parseInlineString("{\n" + code + "\n}", Context.makePosition({min: 0, max: 0, file: file}));
			} catch (e:haxe.macro.Expr.Error) {
				bad++;
				var at = Context.getPosInfos(e.pos).min - 2;
				var src = "{\n" + code;
				var line = src.substr(0, at).split("\n").length - 1;
				var from = src.lastIndexOf("\n", at) + 1;
				var to = src.indexOf("\n", at);
				Sys.println(file + ":" + line + ": " + e.message + "\n\t" + src.substring(from, to < 0 ? src.length : to));
			} catch (e:Dynamic) {
				bad++;
				Sys.println(file + ": " + e);
			}
		}
		Sys.println(files.length + " files, " + bad + " that do not parse");
	}
}
