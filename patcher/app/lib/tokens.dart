import 'package:flutter/widgets.dart';

/// Design tokens shared with the website; the source of truth is design/tokens.json.
const paper = Color(0xfff5f3ed);
const surface = Color(0xfffffefa);
const ink = Color(0xff2a2d2b);
const muted = Color(0xff5c625e);
const line = Color(0xffdedfd6);
const teal = Color(0xff176b5b);
const tealHover = Color(0xff12594b);
const onTeal = Color(0xffffffff);
const mint = Color(0xffdce7d8);
const warning = Color(0xff8a5a00);
const error = Color(0xffa23b2b);

/// The 8-point spacing scale.
const space4 = 4.0;
const space8 = 8.0;
const space16 = 16.0;
const space24 = 24.0;
const space32 = 32.0;
const space40 = 40.0;
const space48 = 48.0;

const radiusSmall = 8.0;
const radiusMedium = 16.0;
const radiusPill = 999.0;

const motionShort = Duration(milliseconds: 160);
const motionMedium = Duration(milliseconds: 240);

/// A transition length, or none when the system asks for reduced motion.
Duration motion(BuildContext context, [Duration length = motionShort]) =>
    MediaQuery.maybeDisableAnimationsOf(context) == true
    ? Duration.zero
    : length;
