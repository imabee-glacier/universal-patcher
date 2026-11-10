import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:patcher/core.dart';
import 'package:patcher/main.dart';
import 'package:patcher/play_button.dart';

import 'snapshots.dart';

class StillCore extends Core {
  @override
  void tick() {}
  @override
  Map<String, dynamic> command(
    String name, [
    Map<String, dynamic> args = const {},
  ]) => {'accepted': true};
}

/// Roboto and the Material icons from the Flutter SDK, so goldens show real text.
Future<void> loadFonts() async {
  final root =
      Platform.environment['FLUTTER_ROOT'] ??
      '${Directory.current.path}/../.tools/flutter';
  final fonts = '$root/bin/cache/artifacts/material_fonts';
  Future<ByteData> read(String name) async => ByteData.sublistView(
    Uint8List.fromList(File('$fonts/$name').readAsBytesSync()),
  );
  final roboto = FontLoader('Roboto');
  for (final face in ['Regular', 'Medium', 'Bold']) {
    roboto.addFont(read('Roboto-$face.ttf'));
  }
  await roboto.load();
  await (FontLoader(
    'MaterialIcons',
  )..addFont(read('MaterialIcons-Regular.otf'))).load();
}

Future<void> screen(
  WidgetTester tester,
  Map<String, dynamic> s,
  String name,
) async {
  tester.view.physicalSize = const Size(900, 640);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.resetPhysicalSize);
  addTearDown(tester.view.resetDevicePixelRatio);
  final core = StillCore()..status.value = s;
  await tester.pumpWidget(PatcherApp(core: core, fontFamily: 'Roboto'));
  await tester.pump(const Duration(milliseconds: 300));
  await expectLater(
    find.byType(PatcherApp),
    matchesGoldenFile('goldens/$name.png'),
  );
}

void main() {
  setUpAll(loadFonts);

  testWidgets('ready screen with Play', (tester) async {
    await screen(
      tester,
      localGame(
        extra: {
          'detail': '58 of 106 packs installed, 71.4 GB on disk.',
          'collection': '58 of 106 packs installed',
        },
      ),
      'ready_play',
    );
  });

  testWidgets('Play unavailable with its reason', (tester) async {
    await screen(
      tester,
      localGame(
        title: 'Your game',
        status: 'Installed',
        play: 'unavailable',
        primary: null,
        reason: 'This is the Windows version of the game, and Wine is not installed, so it cannot be started on this computer.',
        extra: {
          'path': '/home/me/Games/the-sims-4/drive_c/Program Files/EA Games/The Sims 4',
          'detail': '58 of 106 packs installed, 71.4 GB on disk.',
          'collection': '58 of 106 packs installed',
        },
      ),
      'play_disabled',
    );
  });

  testWidgets('smallest window at double text size', (tester) async {
    tester.view.physicalSize = const Size(720, 560);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    final core = StillCore()
      ..status.value = localGame(
        title: 'Pronto para jogar',
        status: 'Pronto para jogar',
        reason: 'O jogo não iniciou. Abra a Steam e tente novamente.',
        extra: {'collection': '58 de 106 pacotes instalados'},
      );
    await tester.pumpWidget(
      MediaQuery(
        data: const MediaQueryData(
          size: Size(720, 560),
          textScaler: TextScaler.linear(2),
        ),
        child: PatcherApp(core: core, fontFamily: 'Roboto'),
      ),
    );
    await tester.pump(const Duration(milliseconds: 300));
    await expectLater(
      find.byType(PatcherApp),
      matchesGoldenFile('goldens/small_window_large_text.png'),
    );
  });

  testWidgets('every Play state', (tester) async {
    tester.view.physicalSize = const Size(560, 700);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    await tester.pumpWidget(
      MaterialApp(
        debugShowCheckedModeBanner: false,
        theme: ThemeData(fontFamily: 'Roboto'),
        home: Scaffold(
          backgroundColor: surface,
          body: Padding(
            padding: const EdgeInsets.all(space32),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                PlayButton(state: PlayState.available, onPressed: () {}),
                const SizedBox(height: space24),
                PlayButton(state: PlayState.starting, onPressed: () {}),
                const SizedBox(height: space24),
                PlayButton(state: PlayState.playing, onPressed: () {}),
                const SizedBox(height: space24),
                PlayButton(
                  state: PlayState.unavailable,
                  reason: 'Confirm your game folder first.',
                  onPressed: () {},
                ),
                const SizedBox(height: space24),
                PlayButton(
                  state: PlayState.available,
                  reason: 'The game did not start. Open Steam, then try again.',
                  onPressed: () {},
                ),
              ],
            ),
          ),
        ),
      ),
    );
    await tester.pump(const Duration(milliseconds: 300));
    await expectLater(
      find.byType(Scaffold),
      matchesGoldenFile('goldens/play_states.png'),
    );
  });
}
