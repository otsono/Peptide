#!/usr/bin/env python3
"""hit_parity.py -- does a converted move hit for what the source's does, on both engines?

`knockback_parity.py` reads the numbers a hit DECLARES and works the launch out from them. This
lands the hit: the other fighter is stood at arm's length on each engine, the move is driven, and
the engine's own report of the hit (`hits`) is read -- what the victim took, and the velocity the
engine gave it on the frame the damage landed.

  damage      the same share of the source's on every move, each dealt fresh on both (the
              port's staling is cleared before every play)
  knockback   the port's over the source's, against the scale a speed converts by

A move that reaches nobody is reported as unlanded, not as a difference. Every wait is on the
engines' own frames.

usage: tools/tests/hit_parity.py fighter [move ...] [--reps=N] [--alone=0]   (a move is this engine's
       name for it; --reps plays a move that landed N times there on each engine and compares the
       usual outcome; a move still apart is played alone on fresh sessions, and listed apart (~) when
       the two agree there, unless --alone=0)
       tools/tests/hit_parity.py --all                (every converted fighter, one at a time)
"""
import glob, json, os, re, sys
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
import verify as V
import trajectory as T

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
OUT = os.path.join(ROOT, "build", "verify")
# each engine driven the way it answers to: the source is told the move by name (it takes it
# through its own name for it, and the same buttons pick a different move there -- a held
# direction with attack is a forward SMASH), this engine is given the buttons (an animation it is
# told to play deals nothing outside the attack it belongs to). what each engine played is read
# back off its own record, and a move the two do not agree on is reported as that.
# a tilt is the way already held as attack comes (pressed together they are a smash on both
# engines: Fighters.smashInput), so it is held long enough first
MOVES = [("jab1", "attack"), ("tilt_forward", "right:6 right+attack"), ("tilt_up", "up:6 up+attack"),
         ("tilt_down", "down:6 down+attack"), ("special_neutral", "special"),
         ("aerial_neutral", "jump:3 attack"), ("aerial_forward", "jump:3 right+attack"),
         ("aerial_back", "jump:3 left+attack"), ("aerial_up", "jump:3 up+attack"),
         ("aerial_down", "jump:3 down+attack"),
         # a throw from a hold, pressed on both engines (the source has no move to name for it)
         ("throw_forward", "grab:2 none:8 right:3"), ("throw_back", "grab:2 none:8 left:3"),
         ("throw_up", "grab:2 none:8 up:3"), ("throw_down", "grab:2 none:8 down:3"),
         # last: a special can leave the fighter changed for what comes after (zelda's down special
         # turns her into sheik on the source, and every move after it was sheik's)
         ("special_side", "right+special"), ("special_down", "down+special")]
# how far away the other fighter stands, in the source's pixels: a move that reaches nobody at one
# distance is tried at the others, and whatever the source landed at is where the port is asked
AT = [34.0, 22.0, 14.0, 48.0, 64.0]
TOL = 0.12          # how far the port's launch may sit off the scale a speed converts by
ANGLE_TOL = 8       # degrees apart the two launches may be read (each is a frame's movement)
HIT = re.compile(r"\+([\d.]+) damage at sample")
KB = re.compile(r"engine knockback ([\d.]+)/frame at (-?[\d.]+) deg")
# how many hits the move landed, and what they came to
COUNT = re.compile(r"HITS:p1 (\d+) hit")
# `HP_RAW=1`: every watch reports each sample (the hit fighter's damage, place and speed) instead
# of its summary, for `rehit_trace.py --victim` to read; nothing is judged then
RAW = " raw" if os.environ.get("HP_RAW") else ""
# the source plays a hop and its aerial as one input sequence, as the port does (see `landed`):
# named after a round trip instead, pikachu's up air began near the end of the hop and landed on
# the way down, where the port's, pressed three frames in, was over before it got there. eight
# fighters' aerials: 9 differ this way against 11 (HP_AIR_SEQ=0 names the move instead)
AIR_SEQ = os.environ.get("HP_AIR_SEQ", "1") == "1" or os.environ.get("HP_AIR_AT") is not None
# HP_AIR_AT=N: the aerial pressed N source frames after the hop's tap, on both engines
AIR_AT = int(os.environ["HP_AIR_AT"]) if os.environ.get("HP_AIR_AT") else None
# grounded moves pressed on the source too, instead of named (see the source's dispatch below)
PRESS = os.environ.get("HP_PRESS") == "1"


# the frames the fighter being played crouches before it leaves the floor, as its stats declare
# (`one_fighter` sets it): an aerial pressed inside them is a grounded move on the source
SQUAT = None


def air_tokens(inp):
    """the move's inputs with its hop held AIR_AT source frames, when that is set; else held past
    the fighter's own jump squat (bowser's is 4 frames, and an up air pressed on the third played
    his up tilt on the source and his up air here)"""
    if AIR_AT is not None:
        return [f"jump:{AIR_AT}" if t.startswith("jump") else t for t in inp.split()]
    out = []
    for t in inp.split():
        if t.startswith("jump") and SQUAT is not None:
            n = int(t.split(":")[1]) if ":" in t else 1
            t = f"jump:{max(n, SQUAT + 1)}"
        out.append(t)
    return out
# how many samples a watch took (the source does not always take all it is asked for)
SAMPLES = re.compile(r"HITS:p1 \d+ hit\(s\) in (\d+) samples")
EACH = re.compile(r"\+([\d.]+) damage at sample (\d+)")
TOTAL = re.compile(r"total ([\d.]+) damage")


def fm_name_of(cid, ssf2_move):
    """this engine's name for a move of the source's, off the conversion's own table"""
    try:
        stats = json.load(open(os.path.join(ROOT, "characters", cid, "conversion_stats.json")))
    except OSError:
        return None
    table = stats.get("ssf2_to_fm_anim") or {}
    name = table.get(ssf2_move)
    return name


def victim_down(eng, per):
    """wait until the one to be hit stands still on the floor: `standing` reads the attacker's
    record, and a victim still coming down from the move before is struck in the air (link's down
    tilt launched an airborne victim 1.6 times as hard as the source's grounded one)"""
    last = None
    for _ in range(20):
        eng.tick(4 * per)
        if isinstance(eng, V.Ssf2Engine):
            try:
                y = V.ssf2_read(eng, "e p1.Y")
            except ValueError:
                continue
            if last is not None and abs(y - last) < 0.01:
                return True
            last = y
        else:
            y = eng.eval("match.getCharacters()[1].getY()")
            if eng.eval("match.getCharacters()[1].isOnFloor()") == "true" and y == last:
                return True
            last = y
    return False


def gap(eng):
    """how far apart the two fighters stand as the move starts, in the source's pixels: both
    engines push two standing fighters apart from where they are put, each by its own amount
    (megaman's water wave struck a victim pushed to 47 here and missed one at 43 in the source),
    so the distance a move is played at is read, not assumed"""
    try:
        if isinstance(eng, V.Ssf2Engine):
            return round(abs(V.ssf2_read(eng, "e p1.X") - V.ssf2_read(eng, "e p0.X")), 1)
        a, b = eng.eval("match.getCharacters()[0].getX()"), eng.eval("match.getCharacters()[1].getX()")
        return round(abs(float(b) - float(a)) / V.SIZE_SCALE, 1)
    except (TypeError, ValueError):
        return None


def push_settled(eng, per):
    """wait until the two fighters stop parting: both engines push two fighters set down inside
    each other apart, this one at once and the source a few pixels a frame, so a move started
    before the source's push is over was played at a distance the source never settles at
    (isaac's gaia, ness's pk flash and zelda's din's fire missed here at the gap the source was
    still passing through)"""
    last = None
    for _ in range(30):
        g = gap(eng)
        if g is not None and last is not None and abs(g - last) < 0.05:
            return g
        last = g
        eng.tick(4 * per)
    return last


def damage_settled(eng, per):
    """wait until nothing an earlier move left is still hurting the one to be hit: a poison
    (black mage's down throw ticks 1 every 15 source frames for 270) outlives the reset on both
    engines and lands its ticks in whatever move is watched next. watched in windows longer than
    a tick's gap until one passes with its damage held"""
    for _ in range(24):
        mark = eng.tell(f"hits p1 {20 * per}")
        text = eng.reply(mark, "HITS:p1", 25, failfast=False)
        if text is None or "none" in text:
            return


def form_base(cid):
    """the fighter whose package another fighter ships in: the source has some fighters only as
    another's final form (giga bowser in bowser's package), and starts them through it"""
    data = json.load(open(os.path.join(ROOT, "build", "verify", "ssf2-data.json")))["character"]
    pkg = data.get(cid)
    others = [k for k, v in data.items() if v == pkg and k != cid]
    return others[0] if len(others) == 1 else None


def take_form(eng):
    """the source's fighter given its final smash and made to use it: the form it turns into
    holds for a few hundred frames, longer than one move"""
    eng.faster(8)

    def transformed():
        m = eng.tell("e p0.TransformedSpecial")
        eng.tick(1)
        return any(l.strip().endswith("true") for l in eng.log()[m:].splitlines() if "<<" in l)
    # a form a reset left running has only what is left of its time, and can run out mid-move:
    # it is waited out and taken afresh
    for _ in range(120):
        if not transformed():
            break
        eng.tick(8)
    eng.tell("e p0.grantFinalSmash()")
    eng.tick(2)
    eng.tell("seq special:2")
    for _ in range(60):
        eng.tick(8)
        if transformed():
            break
    # the change of form plays out before the move is set up
    eng.tick(160)


def at_rate(eng, n):
    """n engine frames a tick: the source sets a move up and settles at eight, and is watched at
    one (its hit watch samples a frame at a time only at its own rate: at eight a 90-frame watch
    covered a third of that, and a late hit or a poison's third tick went unseen)"""
    if n > 1:
        eng.faster(n)
    elif getattr(eng, "speed", 1) > 1:
        eng.reply(eng._raw_tell("speed 1"), "", 10)
        eng.speed = 1


def landed(eng, per, inp, at, hop="short"):
    """(damage, knockback per frame, angle) of the first hit the named move lands, or None"""
    fast = isinstance(eng, V.Ssf2Engine)
    if fast:
        at_rate(eng, 8)
    eng.tell("release all")
    if isinstance(eng, V.Ssf2Engine):
        eng.tell_batch([f"e p0.X = 0", f"e p1.X = {at:.2f}", "reset", "e p0.faceRight()", "e p1.faceLeft()"])
        # a fighter the source has only as another's final form takes that form again (a reset
        # ends it), then stands where the move is played from
        if getattr(eng, "form_of", None):
            take_form(eng)
            eng.tell_batch([f"e p0.X = 0", f"e p1.X = {at:.2f}", "e p0.faceRight()", "e p1.faceLeft()"])
    else:
        # the other fighter faces the one playing the move on both engines: after a reset the source
        # turns it away and this engine toward, and which side of it faces the move decides where
        # the two are pushed apart to and what of it a box reaches
        eng.tell_batch([f"e match.getCharacters()[0].setX(0); match.getCharacters()[1].setX({at:.2f})",
                        "reset", "e match.getCharacters()[0].faceRight(); match.getCharacters()[1].faceLeft()"])
    V.standing(eng, per)
    # the one to be hit back on the attacker's floor: a reset puts it back across, not down, and a
    # launch at damage leaves it on a platform (mario's up tilt at 80% landed it on the left one,
    # and every move after struck the air under it)
    if isinstance(eng, V.Ssf2Engine):
        try:
            eng.tell(f"e p1.Y = {V.ssf2_read(eng, 'e p0.Y'):.2f}")
        except ValueError:
            pass
    else:
        eng.tell("e match.getCharacters()[1].setY(match.getCharacters()[0].getY())")
    victim_down(eng, per)
    push_settled(eng, per)
    damage_settled(eng, per)
    # every move from the same damage on both engines: a launch grows with it, and what the moves
    # before left on the victim is each engine's own (the port's climbed past 80 by the aerials)
    # (HP_DMG starts it elsewhere: a launch's shape can change with damage, as a strong downward
    # hit's bounce does)
    eng.tell(f"dmg p1 {os.environ.get('HP_DMG', '0')}")
    # every move dealt fresh, as the source's first landing of it is: the port stales a move the
    # source's way (Attacks.freshness), and the search here plays a move more times on one engine
    # than the other (landed at another distance first, then tried where the source landed it)
    if not isinstance(eng, V.Ssf2Engine):
        eng.tell("e@p0 self.exports.__usedAttacks = []")
    eng.tick(2 * per)
    # (at the source's own rate from here: at 8 a frame its push has 8 frames to part them again)
    if fast:
        at_rate(eng, 1)
    # the move starts at the distance asked for on both engines: each pushes two standing fighters
    # apart to its own stopping distance (marth's forward tilt started 36 apart on the source and
    # 45 here, so a difference in which box lands read as a spacing one), so the one to be hit is
    # put back where it was asked for once all else has settled, facing kept
    if isinstance(eng, V.Ssf2Engine):
        x0 = V.ssf2_read(eng, "e p0.X")
        eng.tell(f"e p1.X = {x0 + at:.2f}")
    else:
        # and held there: this engine's body push goes on parting the two through the move's
        # start, the source's stops for a fighter attacking (pikachu's thunder fell where the
        # victim had been put, 6 further off by then, and missed it)
        eng.tell(f"e match.getCharacters()[1].setX(match.getCharacters()[0].getX() + {at:.2f}); match.getCharacters()[1].updateGameObjectStats({{solid: false}})")
    eng.tick(1)
    eng.last_gap = gap(eng)
    # a tool's own watcher goes on once the fighters are set down (a `reset` respawns them and
    # drops any timer put on them before it: rehit_trace --proj)
    if getattr(eng, "before_record", None) is not None:
        eng.before_record()
    # which of the attacker's boxes lands each hit, by the source's names, on either engine (the
    # attacker's named value `__peptideHits`, written as each hit lands): which box a difference
    # comes from is the first thing to know about it
    eng.tell(f"gvar p0 {BOX_LOG} 0")
    if not isinstance(eng, V.Ssf2Engine):
        # the distance the two stand at as the attack begins, noted by the runtime (start_gap)
        # and put at the distance asked for on that frame: this engine runs on between the harness's
        # words as the source does, and its push parts the two through them for longer than the
        # source's, so the move started wider here (59 against the source's 51)
        eng.tell(f"e@p0 self.exports.__source.__startGap = null; self.exports.__source.__placeGap = {at:.2f}")
        eng.tell("e@p0 if (AttackRules.__gb0 == null) { AttackRules.__gb0 = AttackRules.began; AttackRules.began = function(me, ?en) { var o = match.getCharacters()[1]; if (me.__placeGap != null) { o.setX(me.__fm.getX() + me.__placeGap); me.__placeGap = null; } if (me.__startGap == null) me.__startGap = o.getX() - me.__fm.getX(); AttackRules.__gb0(me, en); }; }")
    if hop == "drop":
        return with_boxes(eng, dropped(eng, per, inp, at))
    eng.tell("record")
    # on the source, the one to be hit is put at the distance again in the same batch as the move:
    # it runs on between the harness's words and its push parts two standing fighters while it
    # does (put back first, two bowsers asked for 49 began a down tilt 78 apart, and its second
    # kick missed a fighter it strikes from 58)
    place = []
    if isinstance(eng, V.Ssf2Engine):
        try:
            place = [f"e p1.X = {V.ssf2_read(eng, 'e p0.X') + at:.2f}"]
        except ValueError:
            place = []
    if inp[0].startswith("throw"):
        # the source has no grab button: a grab is shield with attack there
        ssf2 = isinstance(eng, V.Ssf2Engine)
        steps = V.fm_steps(inp[1].replace("grab:", "shield+attack:") if ssf2 else inp[1], per, source=True)
        if ssf2:
            eng.tell_batch(place + [f"seq {steps}"])
            mark = eng.tell(f"hits p1 {120 * per}{RAW}")
        else:
            # a throw's presses go with the watch's samples: the hold takes its direction over
            # however long a sample lasts, which is the timing the throws were matched with (as a
            # sequence, a frame apiece, chibirobo's hold took no direction at all)
            mark = eng.tell(f"hits p1 {120 * per} {steps}{RAW}")
    elif isinstance(eng, V.Ssf2Engine):
        # an aerial named on the ground lands at once: a short hop (a tap, released inside the
        # jump's startup) comes first, and the move is named as the fighter leaves the floor
        # (a full hop, held past the startup, for a move that starts too late to land from a
        # short one)
        if AIR_SEQ and any(t.startswith("jump") for t in inp[1].split()):
            # the hop and the aerial in one sequence, frame for frame the port's: a short hop is a
            # tap (released inside the jump's startup) and the rest of the jump's hold is waited
            # out, so the aerial is pressed the same source frame after the tap on both engines
            # (named by `move` after a round trip, lloyd's down air started at the top of the hop
            # here and four frames earlier in the port)
            tokens = []
            for t in air_tokens(inp[1]):
                n = int(t.split(":")[1]) if ":" in t else 1
                if t.startswith("jump") and hop == "short":
                    tokens += ["jump:1"] + ([f"none:{n - 1}"] if n > 1 else [])
                else:
                    tokens.append(t)
            eng.tell_batch(place + [f"seq {' '.join(tokens)}"])
            mark = eng.tell(f"hits p1 {90 * per}{RAW}")
        else:
            if any(t.startswith("jump") for t in inp[1].split()):
                m = eng.tell("seq jump:1" if hop == "short" else "seq jump:4"); eng.await_seq(m, 1)
                eng.tick(3 * per)
            if PRESS and not any(t.startswith("jump") for t in inp[1].split()):
                # pressed, as this engine's is, rather than named: a move whose code reads the
                # buttons as it plays (pac-man's side special steers by the held direction) is the
                # same move only when the same buttons are held (`HP_PRESS=1`)
                eng.tell_batch(place + [f"seq {V.fm_steps(inp[1], 1, source=True)}"])
            else:
                eng.tell_batch(place + [f"move {inp[0]} p0"])
            mark = eng.tell(f"hits p1 {90 * per}{RAW}")
    else:
        # a tapped jump is a short hop on the source; the port names it (see `shorthop`)
        tokens = ["jump+shorthop" + t[len("jump"):] if hop == "short" and (t.startswith("jump:") or t == "jump") else t for t in air_tokens(inp[1])]
        steps = " ".join(f"{t.split(':')[0]}:{int(t.split(':')[1]) * per}" if ":" in t else f"{t}:{per}"
                         for t in tokens)
        if any(t.startswith("jump") for t in inp[1].split()):
            # a hop and its aerial as a sequence, frame for frame the source's: held over the watch's
            # samples, a press lasted as long as a sample took (zelda's up air landed nothing that
            # way). eight fighters' aerials: 6 differed this way against 8
            eng.tell(f"seq {steps}")
            mark = eng.tell(f"hits p1 {90 * per}{RAW}")
        else:
            # a grounded move's presses go with the watch's samples, each held as long as a sample
            # takes, the timing the grounded moves were matched with
            mark = eng.tell(f"hits p1 {90 * per} {steps}{RAW}")
    # the watch runs over the move's own code, and an error that code reports is the move's, not
    # the watch's: the watch still answers (yoshi's back throw landed its 7 after one, and read as
    # landing nothing while a script error ended the wait)
    text = eng.reply(mark, "HITS:p1", 25, failfast=False)
    if text is None:
        return None
    body = eng.log()[mark:]
    dmg, kb = HIT.search(body), KB.search(body)
    if dmg is None:
        return None
    # a hit that launches nothing (krystal's jab: no power, no growth) states no knockback line:
    # it landed, with a launch of 0 and no angle to compare
    kb_at, kb_deg = (float(kb.group(1)), float(kb.group(2))) if kb else (0.0, None)
    # what this engine played for that input, off its own record
    runs = V.trace_runs(eng.read_trace(eng.tell("trace")))
    move = next((a for a, _ in runs if not a.lower().startswith(("stand", "idle", "walk", "fall", "land", "dash", "run", "jump", "grab"))), None)
    count, total = COUNT.search(body), TOTAL.search(body)
    # each hit as (source frame after the first, damage): this engine samples two frames a source one
    each = [(int(m.group(2)) // per, float(m.group(1))) for m in EACH.finditer(body)]
    # how long the watch went on after the first hit, in source frames
    watched = SAMPLES.search(body)
    after = (int(watched.group(1)) // per - each[0][0]) if (watched and each) else None
    each = [(f - each[0][0], d) for f, d in each] if each else []
    return with_boxes(eng, (float(dmg.group(1)), kb_at, kb_deg, move,
            int(count.group(1)) if count else 1, float(total.group(1)) if total else float(dmg.group(1)), each, after))


BOX_LOG = "__peptideHits"


def start_gap(eng):
    """how far apart the two stood as the attacker's move began, in the source's pixels, off each
    engine's own record of that frame: both engines run on between the harness's words, and the
    source's push goes on parting two standing fighters while they do (two bowsers put back 34 apart
    stood 66 apart as a down tilt began), so a read made before the move is not the move's distance.
    the source's record carries both fighters a frame; this engine's runtime notes it as the attack
    begins (the hook put in by `landed`)"""
    try:
        if isinstance(eng, V.Ssf2Engine):
            mark = eng.tell("trace raw all")
            eng.tick(2)
            att, first = {}, None
            for line in eng.log()[mark:].splitlines():
                if "\t" not in line:
                    continue
                f = line.split("\t", 1)[1].split("|")
                if len(f) < 11:
                    continue
                who, frame = f[9].strip(), f[10].strip()
                if who == "0":
                    att[frame] = (f[0].strip().lower(), float(f[2]))
                elif who == "1" and frame in att:
                    label, ax = att[frame]
                    if first is None:
                        first = label
                    # the first frame the attacker is in its move: a label that is not one of the
                    # neutral ones (an idle turns into another idle), matched whole, since a
                    # move's own can begin with a neutral one's (bowser's down tilt is crouch_attack)
                    if label != first and label.split("_")[0] not in ("stand", "idle", "walk", "land", "fall") and label not in ("crouch", "jump"):
                        return round(abs(float(f[2]) - ax), 1)
            return None
        v = eng.eval("match.getCharacters()[0].exports.__source.__startGap")
        v = v.split(":", 2)[-1].strip() if isinstance(v, str) else v
        return None if v in (None, "null", "") else round(abs(float(v)) / V.SIZE_SCALE, 1)
    except (TypeError, ValueError):
        return None


def with_boxes(eng, hit):
    """the outcome with the boxes that landed it added (an empty list where the log is unread: a
    fighter converted before the log existed)"""
    if hit is None:
        return None
    g = start_gap(eng)
    if g is not None:
        eng.last_gap = g
    mark = eng.tell(f"gvar p0 {BOX_LOG}")
    eng.tick(2)
    boxes = []
    for line in eng.log()[mark:].splitlines():
        if "<<" not in line or ("GVAR:" not in line and "E:" not in line):
            continue
        v = line.split("=", 1)[1] if "GVAR:" in line else line.split("E:", 1)[1]
        boxes = [b for b in v.strip().split(",") if b.startswith("attackBox")]
        break
    return tuple(hit) + (boxes,)


# how far above the floor an aerial starts in the `drop` hop, in the source's pixels
DROP = 80


def floor_of(eng, y):
    """the height the fighter stands at, from the lowest it has been seen standing: the source's
    fighter is not always back on the floor between tries (a drop measured from 38 pixels up fell
    three frames and landed its landing box), so a drop is measured from the floor, not from it"""
    low = getattr(eng, "_floor_y", None)
    eng._floor_y = y if low is None else max(low, y)
    return eng._floor_y


def dropped(eng, per, inp, at=None):
    """an aerial started from rest DROP pixels above the floor, the same on both engines: a hop
    reaches the move at each engine's own point of the jump, and a different box lands. the
    fighter is parked and the buttons pressed in one batch, so the fall before the move is counted
    in engine frames rather than round trips (the park is the C6 gate's: this engine moves before it
    adds gravity and the source after, so it starts with a step and a half of fall speed. two and a
    half matches samus's fall frame for frame, yet 12 more roster aerials differed with it: the
    step and a half is what the roster's aerials land with)"""
    buttons = " ".join(t for t in inp[1].split() if not t.startswith("jump"))
    # a back aerial is aimed at the victim: the attacker turned away from it, the stick the other way
    back = "left" in buttons.split("+")[0] or "left+" in buttons
    if back:
        buttons = buttons.replace("left", "right")
    steps = V.fm_steps(buttons, per, source=True)
    if isinstance(eng, V.Ssf2Engine):
        y = floor_of(eng, V.ssf2_read(eng, "e p0.Y"))
        # turned first, on its own: the source takes the batch's commands a frame apiece, so a
        # turn between the park and the press was another frame falling before a back aerial,
        # which reached the fighter lower and later than every other aerial (ness's landed its
        # landing box)
        if back:
            eng.tell("e p0.faceLeft()")
            eng.tick(per)
        # the one to be hit put at the distance again in the same batch as the drop, as for every
        # other move: the source's push goes on parting two standing fighters through the round
        # trips before it (captain falcon's down air, put 34 apart, began 52.8 apart)
        place = [] if at is None else [f"e p1.X = {V.ssf2_read(eng, 'e p0.X') + at:.2f}"]
        eng.tell_batch(place + [f"e p0.Y = {y - DROP:.2f}", "record", f"seq {steps}"])
        mark = eng.tell(f"hits p1 {90 * per}{RAW}")
    else:
        c = "match.getCharacters()[0]"
        y = eng.eval(f"{c}.getY()")
        # an engine that stops answering lands nothing here (the sweep goes on to the next move)
        if y is None:
            return None
        y = floor_of(eng, float(y))
        # into the fall first: entering the state takes the speed off, and a speed set before it
        # left the fighter a frame still and its move half a source frame late (mario's back air
        # passed over the fighter it lands on in the source)
        park = (f"e {c}.toState(CState.FALL); {c}.setY({y - DROP * V.SIZE_SCALE:.1f}); "
                f"{c}.setYVelocity({c}.getCharacterStat(\"gravity\") * 1.5)" + (f"; {c}.faceLeft()" if back else ""))
        # the presses as a sequence in the park's batch, frame for frame the source's: held over
        # the watch's samples instead, a sample can take more than a frame, and the stick a back
        # aerial holds toward the fighter went on pushing it there (pac-man drifted 18 across where
        # the source's one frame of it drifted 7). eight fighters' aerials from the drop: 5
        # differed this way against 11
        eng.tell_batch([park, "record", f"seq {V.fm_steps(buttons, per)}"])
        mark = eng.tell(f"hits p1 {90 * per}{RAW}")
    # the watch runs over the move's own code, and an error that code reports is the move's, not
    # the watch's: the watch still answers (yoshi's back throw landed its 7 after one, and read as
    # landing nothing while a script error ended the wait)
    text = eng.reply(mark, "HITS:p1", 25, failfast=False)
    if text is None:
        return None
    body = eng.log()[mark:]
    dmg, kb = HIT.search(body), KB.search(body)
    if dmg is None:
        return None
    # a hit that launches nothing (krystal's jab: no power, no growth) states no knockback line:
    # it landed, with a launch of 0 and no angle to compare
    kb_at, kb_deg = (float(kb.group(1)), float(kb.group(2))) if kb else (0.0, None)
    runs = V.trace_runs(eng.read_trace(eng.tell("trace")))
    move = next((a for a, _ in runs if not a.lower().startswith(("stand", "idle", "walk", "fall", "land", "dash", "run", "jump", "grab"))), None)
    count, total = COUNT.search(body), TOTAL.search(body)
    # each hit as (source frame after the first, damage): this engine samples two frames a source one
    each = [(int(m.group(2)) // per, float(m.group(1))) for m in EACH.finditer(body)]
    # how long the watch went on after the first hit, in source frames
    watched = SAMPLES.search(body)
    after = (int(watched.group(1)) // per - each[0][0]) if (watched and each) else None
    each = [(f - each[0][0], d) for f, d in each] if each else []
    return (float(dmg.group(1)), kb_at, kb_deg, move,
            int(count.group(1)) if count else 1, float(total.group(1)) if total else float(dmg.group(1)), each, after)


# how many times a move that landed is played again where it landed (`--reps=N`): the source
# itself lands some moves differently run to run (lloyd's down special lands its start-up box in
# one run and only its dive in another), so the outcome compared is the one most of them show
REPS = next((int(a.split("=", 1)[1]) for a in sys.argv if a.startswith("--reps=")), 1)
# how many times a hit count that differs is played again on the source to see its own spread
SPREAD = next((int(a.split("=", 1)[1]) for a in sys.argv if a.startswith("--spread=")), 3)
# a move that still differs is played once more on each engine in a session of its own (`--alone=0`
# for none): what the moves before it leave behind can part the two (isaac's down special spawns
# its pillar off his own speed, and the move before left him sliding differently on each)
ALONE = next((int(a.split("=", 1)[1]) for a in sys.argv if a.startswith("--alone=")), 1)


def agree(a, b, share, scale):
    """whether two plays of a move (the source's, this engine's) land the same hit: its damage at
    the share the other moves pay, its launch at the scale, its angle, its count"""
    if a is None or b is None:
        return False
    if share and a[0] and abs(b[0] / a[0] - share) > 0.03 * share and abs(b[0] - a[0] * share) > 0.15:
        return False
    if a[1] and abs(b[1] / a[1] - scale) > TOL * scale:
        return False
    if a[2] is not None and b[2] is not None and abs((a[2] - b[2] + 180) % 360 - 180) > ANGLE_TOL:
        return False
    return a[4] == b[4]


def usual(eng, per, inp, at, hop, first):
    """the outcome the move shows most often over REPS plays where it first landed (its hit count,
    first hit's damage and angle); with one play, that play"""
    if REPS <= 1:
        return first
    plays = [first] + [h for h in (landed(eng, per, inp, at, hop) for _ in range(REPS - 1)) if h is not None]
    key = lambda h: (h[4], round(h[0]), None if h[2] is None else round(h[2] / 5))
    counts = {}
    for h in plays:
        counts[key(h)] = counts.get(key(h), 0) + 1
    best = max(counts, key=lambda k: counts[k])
    return next(h for h in plays if key(h) == best)


def roster():
    return sorted(json.load(open(os.path.join(ROOT, "build", "verify", "ssf2-data.json")))["character"])


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("-")]
    if "--all" in sys.argv:
        # one fighter at a time: the engines hold one match each, and a sweep is one after another
        worst = 0
        for cid in (args or roster()):
            # one fighter whose engine gives out is reported, and the sweep goes on to the next
            try:
                worst = max(worst, one_fighter(cid, []))
            except Exception as e:
                print(f"{cid}: the run stopped ({type(e).__name__}: {e})")
                worst = max(worst, 1)
        return worst
    if not args:
        print(__doc__)
        return 2
    return one_fighter(args[0], args[1:])


def squat_of(cid):
    """the jump squat the fighter's stats declare (`jumpStartup`), off its converted script"""
    for f in glob.glob(os.path.join(ROOT, "characters", cid, "library", "scripts", "*", "Script.hx")):
        m = re.search(r"jumpStartup: (\d+)", open(f, encoding="utf8", errors="replace").read())
        if m:
            return int(m.group(1))
    return None


def one_fighter(cid, named_moves):
    global SQUAT
    SQUAT = squat_of(cid)
    # the package the engine loads is built from what is in `characters/`, so build it first: a
    # hit read off a stale package is a measurement of the last conversion
    project = next(iter(glob.glob(os.path.join(ROOT, "characters", cid, "*.fraytools"))), None)
    if project and "--no-pack" not in sys.argv:
        import subprocess
        subprocess.run([os.path.join(ROOT, "build", "release", "peptide"), "pack", "--project", project],
                       capture_output=True, check=False)
    # a move named on the command line is played by its inputs, as the sweep plays it (a name
    # the sweep does not know is taken as the inputs themselves)
    known = dict(MOVES)
    moves = [(m, known.get(m, m)) for m in named_moves] or MOVES
    named = {m: pair for m, pair in zip([p[0] for p in moves], moves)}
    scale = V.SIZE_SCALE / 2          # a speed a frame here against one a source frame
    s = V.Ssf2Engine(f"hits-ssf2-{cid}")
    base = None
    if s.boot(["--char", f"{cid},{cid}", "--stage", V.CHAR_STAGE_SSF2]) is None or s.wait(("ANIM:",), 120) is None:
        # one the source cannot start as itself may be another fighter's final form
        base = form_base(cid)
        s.close(final=True)
        if base:
            print(f"{cid} is played on the source as {base}'s final form, against {base}")
            s = V.Ssf2Engine(f"hits-ssf2-{cid}")
        if not base or s.boot(["--char", f"{base},{base}", "--stage", V.CHAR_STAGE_SSF2]) is None or s.wait(("ANIM:",), 120) is None:
            print("the SSF2 session never came up")
            return 1
    s.form_of = base
    T.entrance_over(s, ("ANIM:ENTRANCE",))
    src, reach, hop_of, src_gap = {}, {}, {}, {}
    for m, _ in moves:
        hops = ["drop", "short", "full"] if any(t.startswith("jump") for t in named[m][1].split()) else ["short"]
        for hop in hops:
            for at in AT:
                hit = landed(s, 1, named[m], at, hop)
                # the source's grounded moves land every other try at a distance they reach each
                # time (goku's grab and up tilt at 22 to 34: a missed up tilt started from another
                # height, landed early and was cut short), so one is tried twice before the
                # distance is given up
                if hit is None and hop != "drop" and not any(t.startswith("jump") for t in named[m][1].split()):
                    hit = landed(s, 1, named[m], at, hop)
                if hit is not None:
                    src_gap[m] = getattr(s, "last_gap", None)
                    src[m], reach[m], hop_of[m] = usual(s, 1, named[m], at, hop, hit), at, hop
                    break
            if m in src:
                break
    s.close(final=True)
    port, port_reach, port_gap = {}, {}, {}

    def put(m, at):
        # the port's victim is put where the source's stood as the move started (the source
        # pushes two standing fighters apart further than asked, bowsers 34 apart to 55, and
        # that distance can't be held there): at the source's reach, its played distance
        g = src_gap.get(m)
        return (g if at == reach.get(m) and g else at) * V.SIZE_SCALE

    def play_port(f, m):
        # where the source landed it first, then the other distances: a move whose reach differs
        # is a difference worth naming, not a move that landed nothing. an aerial is also tried
        # from the other hop: each engine reaches the move at its own point of the hop, and what
        # is compared is the box the source landed (its damage at the share the moves pay), so
        # the first landing on that box is the one taken, else the first landing at all
        aerial = any(t.startswith("jump") for t in named[m][1].split())
        hops = [hop_of.get(m, "short")] + ([h for h in ("drop", "short", "full") if h != hop_of.get(m, "short")] if aerial else [])
        first = None
        for hop in hops:
            # a grounded move is tried at the distances in the source's own order: a move that
            # changes as it is used (sora's down special cycles fire, blizzard, thunder) is then
            # played as many times on both before it lands
            order = [reach[m]] + [a for a in AT if a != reach[m]] if aerial else AT
            for at in order:
                hit = landed(f, 2, named[m], put(m, at), hop)
                # tried twice at a distance, as the source's are
                if hit is None and not aerial:
                    hit = landed(f, 2, named[m], put(m, at), hop)
                if hit is None:
                    continue
                if first is None:
                    first = (hit, at)
                if not aerial or abs(hit[0] / src[m][0] - 1.0) < 0.03:
                    port_gap[m] = getattr(f, "last_gap", None)
                    port[m], port_reach[m] = usual(f, 2, named[m], put(m, at), hop, hit), at
                    break
            if m in port:
                break
        if m not in port and first is not None:
            port[m], port_reach[m] = first
        # landed first at another distance: tried where the source landed it before a reach is
        # named a difference (a move at the edge of its reach lands there some plays and not
        # others on both engines, and the two searches stop at different distances)
        if m in port and port_reach.get(m) != reach[m] and not aerial:
            for _ in range(2):
                hit = landed(f, 2, named[m], put(m, reach[m]), hop_of.get(m, "short"))
                if hit is not None:
                    port_gap[m] = getattr(f, "last_gap", None)
                    port[m], port_reach[m] = usual(f, 2, named[m], put(m, reach[m]), hop_of.get(m, "short"), hit), reach[m]
                    break

    def fresh_port(tag):
        f = V.Engine(f"hits-fm-{cid}{tag}")
        if f.boot(["--char", f"{cid},{base or cid}", "--stage", V.CHAR_STAGE_FM]) is None:
            return None
        T.entrance_over(f, ("ANIM:INTRO",))
        f.tick(30)
        return f

    f = fresh_port("")
    if f is None:
        print("the Fraymakers session never came up")
        return 1
    for m, _ in moves:
        if m in reach:
            play_port(f, m)
    f.close(final=True)
    # a move the source lands and this engine missed is played once more in a session of its own:
    # what the moves before it leave behind can stop it (pichu's three throws landed nothing in
    # the sweep and matched the source played alone)
    again = [m for m, _ in moves if m in reach and m not in port]
    if again:
        f = fresh_port("-again")
        if f is not None:
            for m in again:
                play_port(f, m)
            f.close(final=True)
    # a move each engine landed first at another distance: the source is tried where this engine
    # landed it. a move at the edge of its reach lands there some plays and not others on both
    # engines (the sweep to sweep reach lines flipped by the dozen), so the reach is only named a
    # difference when the source does not land it where this engine did
    both_reach = set()
    apart = [m for m in port if m in reach and port_reach.get(m) != reach.get(m)]
    if apart:
        s = V.Ssf2Engine(f"hits-ssf2-{cid}-reach")
        if s.boot(["--char", f"{base or cid},{base or cid}", "--stage", V.CHAR_STAGE_SSF2]) is not None and s.wait(("ANIM:",), 120) is not None:
            s.form_of = base
            T.entrance_over(s, ("ANIM:ENTRANCE",))
            for m in apart:
                for _ in range(2):
                    if landed(s, 1, named[m], port_reach[m], hop_of.get(m, "short")) is not None:
                        both_reach.add(m)
                        break
        s.close(final=True)
    report = {"fighter": cid, "moves": {}, "problems": [], "unlanded": [], "edges": [], "spacing": []}
    for m, _ in moves:
        a, b = src.get(m), port.get(m)
        if a is None or b is None:
            # where the source landed one this engine did not: the hop and distance to play it from
            where = f" (the source lands it from a {hop_of.get(m)} hop at {reach.get(m)}, for {a[0]})" if a is not None else ""
            report["unlanded"].append(f"{m}: {'source' if a is None else 'port'} landed nothing{where}")
            continue
        if port_reach.get(m) != reach.get(m):
            line = f"{m}: the source lands it at {reach.get(m)} away, this engine at {port_reach.get(m)}"
            # listed apart when the source lands it where this engine did too: the move is at the
            # edge of this engine's reach at the source's distance, not out of it
            if m in both_reach:
                report["edges"].append(f"{line} (the source lands it at {port_reach.get(m)} too)")
            else:
                report["problems"].append(line)
        # the same move on both, or the comparison is between two different moves
        want = fm_name_of(cid, a[3])
        same = want is not None and b[3] is not None and (b[3] == want
            or (b[3].startswith(want) and b[3][len(want):].lstrip("_").isdigit()))
        if not same:
            report["problems"].append(f"{m}: the source plays {a[3]}, this engine {b[3]}")
        # the distance each engine actually played it at, once both had pushed their fighters apart
        ga, gb = src_gap.get(m), port_gap.get(m)
        if os.environ.get("HP_GAPS"):
            report["spacing"].append(f"{m}: started {ga} apart on the source, {gb} here")
        elif ga is not None and gb is not None and abs(ga - gb) >= 3:
            report["spacing"].append(f"{m}: played {ga} apart on the source, {gb} here")
        report["moves"][m] = {"stood": reach.get(m), "stood_here": port_reach.get(m), "hop": hop_of.get(m), "both_reach": m in both_reach,
                              "gap": ga, "gap_here": gb,
                              "source": {"damage": a[0], "knockback": a[1], "angle": a[2], "move": a[3], "hits": a[4], "total": a[5], "each": a[6], "after": a[7], "boxes": a[8] if len(a) > 8 else []},
                              "port": {"damage": b[0], "knockback": b[1], "angle": b[2], "move": b[3], "hits": b[4], "total": b[5], "each": b[6], "after": b[7], "boxes": b[8] if len(b) > 8 else []},
                              "same_move": same,
                              "damage_ratio": round(b[0] / a[0], 3) if a[0] else None,
                              "knockback_ratio": round(b[1] / a[1], 3) if a[1] else None}
    # the share the port pays of the source's damage, as the moves themselves show it: a move off
    # what the rest agree on is the one that differs, and the share itself is the engines' own
    shares = sorted(r["damage_ratio"] for r in report["moves"].values() if r["damage_ratio"])
    report["damage_share"] = shares[len(shares) // 2] if shares else None
    for m, row in report["moves"].items():
        share, r = report["damage_share"], row["knockback_ratio"]
        if not row.get("same_move", True):
            continue
        # off the share by more than 3%, and by more than a tenth of a point of damage: each engine
        # states a hit to a tenth, and a fresh 3 (3.15) reads 3.2 there and 3.1 here, which is
        # rounding, not a different hit
        want_dmg = row["source"]["damage"] * share if share else None
        if share and row["damage_ratio"] and abs(row["damage_ratio"] - share) > 0.03 * share \
                and abs(row["port"]["damage"] - want_dmg) > 0.15:
            report["problems"].append(
                f"{m}: {row['source']['damage']} damage on the source, {row['port']['damage']} here "
                f"({row['damage_ratio']} of it, the other moves pay {share})")
        # a fixed launch is converted for a victim of the roster's median weight (hitbox_stats.jsonc
        # `fixed_knockback`), which a mirror match's victim is near enough for the scale to hold
        want = scale
        row["expected_ratio"] = round(want, 3)
        if r is not None and abs(r - want) > TOL * want:
            report["problems"].append(f"{m}: launch {row['source']['knockback']:.2f} a source frame against "
                                      f"{row['port']['knockback']:.2f} here (ratio {r}, expected {want:.3f})")
        # and the way it sends the victim: a box sent the mirrored way lands with the same strength
        # and was invisible to the checks above (eighteen up tilts leaned forward instead of back)
        sa, pa = row["source"]["angle"], row["port"]["angle"]
        if sa is not None and pa is not None and abs((sa - pa + 180) % 360 - 180) > ANGLE_TOL:
            report["problems"].append(f"{m}: sent at {sa:.0f} degrees on the source, {pa:.0f} here")
        # and how many times it hits: a move that hits again on the source (its refresh, a box
        # that comes back) landing once here deals a share of its damage, and the first hit
        # alone cannot tell
        sh, ph = row["source"].get("hits"), row["port"].get("hits")
        # only the hits both watches saw: the source's watch can end sooner than this engine's (it
        # took about fifty samples of the ninety asked), and a hit after it is not compared
        cut = row["source"].get("after")
        if cut is not None and row["port"].get("each"):
            seen = [d for f, d in row["port"]["each"] if f <= cut]
            ph = len(seen)
            row["port"]["total"] = round(sum(seen), 1)
        row["port"]["hits_seen"] = ph
        if sh is not None and ph is not None and sh != ph:
            report["problems"].append(f"{m}: hits {sh} time(s) for {row['source']['total']:.1f} on the source, "
                                      f"{ph} for {row['port']['total']:.1f} here")
    # a launch or angle apart for a reason already accepted is listed apart, not as a difference:
    # a box aimed down at the standing fighter bounces it with another share of the launch than the
    # source keeps (STATUS: the downward-hit gap), and an autolink's launch follows the attacker's
    # own motion, which the two engines' inputs give it differently
    # a move's own differences carry the boxes that landed it on each engine, in order: a hit the
    # source lands with its second box and this engine with its first is a reach or a spacing, and
    # the two lists side by side say where a count parts
    def with_box_names(line):
        m, what = line.split(": ", 1)
        row = report["moves"].get(m)
        if row is None or not what.startswith(("hits ", "sent at ", "launch ")) and " damage on the source" not in what:
            return line
        sb, pb = row["source"].get("boxes") or [], row["port"].get("boxes") or []
        if not sb and not pb:
            return line
        short = lambda bs: " ".join(b.replace("attackBox", "b") or "b" for b in bs) or "?"
        return f"{line} [boxes: source {short(sb)}; here {short(pb)}]"
    report["problems"] = [with_box_names(line) for line in report["problems"]]
    kinds = declared_kinds(cid)
    report["accepted"] = []
    kept = []
    for line in report["problems"]:
        m, what = line.split(": ", 1)
        why = None
        if what.startswith(("launch ", "sent at ")):
            k = kinds.get(m, set())
            why = "autolink" if "autolink" in k else "a downward box on a standing fighter" if "down" in k else None
        if why:
            report["accepted"].append(f"{line} ({why})")
        else:
            kept.append(line)
    report["problems"] = kept
    # a hit count the source itself lands more than one way: the move played again on the source
    # where it landed it (`--spread=N`, 3 by default, 0 for none), and a count this engine's play
    # matches one of is the source's own spread, listed apart (meta knight's tornado lands 9 or 10
    # as its victim drifts through the box, run to run)
    report["spread"] = []
    spread_moves = [line.split(": ", 1)[0] for line in report["problems"] if ": hits " in line]
    if SPREAD > 0 and spread_moves:
        s = V.Ssf2Engine(f"hits-ssf2-{cid}")
        if s.boot(["--char", f"{base or cid},{base or cid}", "--stage", V.CHAR_STAGE_SSF2]) is not None \
                and s.wait(("ANIM:",), 120) is not None:
            s.form_of = base
            T.entrance_over(s, ("ANIM:ENTRANCE",))
            for m in spread_moves:
                counts = []
                for _ in range(SPREAD):
                    h = landed(s, 1, named[m], reach[m], hop_of.get(m, "short"))
                    if h is not None:
                        counts.append(h[4])
                ph = report["moves"][m]["port"].get("hits_seen")
                if ph in counts:
                    line = next(l for l in report["problems"] if l.startswith(f"{m}: hits "))
                    report["problems"].remove(line)
                    report["spread"].append(f"{line} (the source landed it {sorted(counts)} times over {SPREAD} plays)")
        s.close(final=True)
    # each move still apart, played alone on a fresh session of each engine where the source
    # landed it: one that agrees there was parted by what the sweep's moves before it left behind,
    # and is listed apart with what each engine showed alone
    report["alone"] = []
    alone_moves = []
    for line in report["problems"]:
        m = line.split(": ", 1)[0]
        if m in report["moves"] and m not in alone_moves and report["moves"][m].get("same_move", True):
            alone_moves.append(m)
    if ALONE > 0 and alone_moves:
        for m in alone_moves:
            s = V.Ssf2Engine(f"hits-ssf2-{cid}-alone")
            a = None
            if s.boot(["--char", f"{base or cid},{base or cid}", "--stage", V.CHAR_STAGE_SSF2]) is not None \
                    and s.wait(("ANIM:",), 120) is not None:
                s.form_of = base
                T.entrance_over(s, ("ANIM:ENTRANCE",))
                a = landed(s, 1, named[m], reach[m], hop_of.get(m, "short"))
            s.close(final=True)
            f = fresh_port("-alone")
            b = None
            if f is not None:
                b = landed(f, 2, named[m], put(m, reach[m]), hop_of.get(m, "short"))
                f.close(final=True)
            if agree(a, b, report["damage_share"], scale):
                gone = [l for l in report["problems"] if l.split(": ", 1)[0] == m]
                for l in gone:
                    report["problems"].remove(l)
                    report["alone"].append(f"{l} (played alone on each engine they agree: {a[0]} damage, {a[4]} hit(s) on the source; {b[0]}, {b[4]} here)")
    os.makedirs(OUT, exist_ok=True)
    json.dump(report, open(os.path.join(OUT, f"hits-{cid}.json"), "w"), indent=1)
    print(f"{cid}: {len(report['moves'])} move(s) landed on both, {len(report['unlanded'])} unlanded, "
          f"{len(report['problems'])} difference(s)"
          + (f", the port pays {report['damage_share']} of the source's damage" if report["damage_share"] else ""))
    for line in report["unlanded"] + report["problems"]:
        print(f"  {line}")
    for line in report["accepted"]:
        print(f"  ~ {line}")
    for line in report["spread"]:
        print(f"  ~ {line}")
    for line in report["alone"]:
        print(f"  ~ {line}")
    for line in report["edges"]:
        print(f"  ? {line}")
    for line in report["spacing"]:
        print(f"  = {line}")
    return 1 if report["problems"] else 0


def declared_kinds(cid):
    """what each move's boxes declare that a known gap turns on, off the converted HitboxStats:
    {move: {"down", "autolink"}}"""
    out = {}
    for path in glob.glob(os.path.join(ROOT, "characters", cid, "library", "scripts", "*", "HitboxStats.hx")):
        cur = None
        for line in open(path):
            mm = re.match(r"^\t(\w+): \{\s*$", line)
            if mm:
                cur = mm.group(1)
                continue
            if cur is None or "hitbox" not in line:
                continue
            k = out.setdefault(cur, set())
            if "AUTOLINK" in line:
                k.add("autolink")
            a = re.search(r"angle: (-?[\d.]+)", line)
            if a and 180 < float(a.group(1)) % 360 < 360:
                k.add("down")
    return out


if __name__ == "__main__":
    sys.exit(main())
