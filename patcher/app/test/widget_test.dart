import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:patcher/collection.dart';
import 'package:patcher/core.dart';
import 'package:patcher/main.dart';

import 'snapshots.dart';

class FakeCore extends Core {
  final commands = <String>[];
  final arguments = <Map<String, dynamic>>[];
  @override
  void tick() {}
  @override
  Map<String, dynamic> command(
    String name, [
    Map<String, dynamic> args = const {},
  ]) {
    commands.add(name);
    arguments.add(args);
    return {'accepted': true};
  }
}

Finder get playButton => find.byKey(const Key('play-button'));

Future<FakeCore> show(WidgetTester tester, Map<String, dynamic> s) async {
  final core = FakeCore()..status.value = s;
  await tester.pumpWidget(PatcherApp(core: core));
  await tester.pump();
  return core;
}

Future<void> settle(WidgetTester tester) async {
  await tester.pump();
  await tester.pump(const Duration(seconds: 1));
}

void main() {
  group('Play button', () {
    testWidgets('starts the game when the engine offers Play', (tester) async {
      final core = await show(tester, localGame());
      expect(find.text('Play'), findsOneWidget);
      expect(find.text('Ready to play'), findsWidgets);
      await tester.tap(playButton);
      expect(core.commands, ['Launch']);
      // One Play button only: it is not repeated in the card.
      expect(find.byKey(const Key('action-Launch')), findsNothing);
      expect(find.byType(FilledButton), findsNothing);
    });

    testWidgets('starting shows a ring and does not start twice', (
      tester,
    ) async {
      final core = await show(
        tester,
        localGame(
          play: 'starting',
          status: 'Starting',
          reason: 'The game may need a moment to open.',
        ),
      );
      expect(find.text('Starting...'), findsOneWidget);
      expect(find.byKey(const Key('play-progress')), findsOneWidget);
      expect(find.text('The game may need a moment to open.'), findsOneWidget);
      await tester.tap(playButton, warnIfMissed: false);
      expect(core.commands, isEmpty);
    });

    testWidgets('playing is calm, shows a live dot and cannot be pressed', (
      tester,
    ) async {
      final core = await show(
        tester,
        localGame(
          play: 'playing',
          status: 'Playing',
          actions: ['SetPolicy', 'ViewPacks'],
          unavailable: {'Verify': 'Close the game first.'},
        ),
      );
      expect(find.text('Playing'), findsWidgets);
      expect(find.byKey(const Key('play-live-dot')), findsOneWidget);
      expect(find.byKey(const Key('action-Verify')), findsNothing);
      await tester.tap(playButton, warnIfMissed: false);
      expect(core.commands, isEmpty);
      await tester.pumpWidget(const SizedBox());
    });

    testWidgets('unavailable is muted and says why under the button', (
      tester,
    ) async {
      const reason =
          'This is the Windows version of the game, and Wine is not installed, so it cannot be started on this computer.';
      final core = await show(
        tester,
        localGame(
          play: 'unavailable',
          reason: reason,
          title: 'Your game',
          status: 'Installed',
          primary: null,
        ),
      );
      expect(find.text(reason), findsOneWidget, reason: 'not only a tooltip');
      expect(
        find.descendant(
          of: find.byKey(const Key('play-reason')),
          matching: find.text(reason),
        ),
        findsOneWidget,
      );
      await tester.tap(playButton, warnIfMissed: false);
      expect(core.commands, isEmpty);
    });

    testWidgets('a failed start keeps Play and names the next action', (
      tester,
    ) async {
      final core = await show(
        tester,
        localGame(
          reason: 'The game did not start. Open Steam, then try again.',
        ),
      );
      expect(
        find.text('The game did not start. Open Steam, then try again.'),
        findsOneWidget,
      );
      await tester.tap(playButton);
      expect(core.commands, ['Launch']);
    });

    testWidgets('Enter and Space press it from the keyboard', (tester) async {
      final core = await show(tester, localGame());
      final inner = find.descendant(
        of: playButton,
        matching: find.byType(GestureDetector),
      );
      Focus.of(tester.element(inner)).requestFocus();
      await tester.pump();
      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      await tester.sendKeyEvent(LogicalKeyboardKey.space);
      await tester.pump();
      expect(core.commands, ['Launch', 'Launch']);
      // The keyboard focus ring is drawn.
      final ring = tester
          .widgetList<AnimatedContainer>(
            find.descendant(
              of: playButton,
              matching: find.byType(AnimatedContainer),
            ),
          )
          .map((c) => c.decoration)
          .whereType<BoxDecoration>()
          .where((d) => d.border != null);
      expect(ring.any((d) => (d.border! as Border).top.color == ink), isTrue);
    });

    testWidgets('reduced motion: no spinner, no pulse, changes at once', (
      tester,
    ) async {
      final core = FakeCore()..status.value = localGame();
      Widget app() => MediaQuery(
        data: const MediaQueryData(
          size: Size(800, 600),
          disableAnimations: true,
        ),
        child: PatcherApp(core: core),
      );
      await tester.pumpWidget(app());
      core.status.value = localGame(play: 'starting', status: 'Starting');
      await tester.pump();
      await tester.pump(Duration.zero);
      expect(find.text('Starting...'), findsOneWidget);
      expect(find.text('Play'), findsNothing, reason: 'no cross-fade');
      expect(find.byType(CircularProgressIndicator), findsNothing);
      expect(find.byIcon(Icons.hourglass_empty_rounded), findsOneWidget);
      core.status.value = localGame(play: 'playing', status: 'Playing');
      await tester.pump();
      await tester.pump(Duration.zero);
      final dot = tester.widget<FadeTransition>(
        find
            .ancestor(
              of: find.byKey(const Key('play-live-dot')),
              matching: find.byType(FadeTransition),
            )
            .first,
      );
      await tester.pump(const Duration(milliseconds: 800));
      expect(dot.opacity.value, 1, reason: 'the live dot does not pulse');
      await tester.pumpWidget(const SizedBox());
    });
  });

  testWidgets('the smallest window at double text size keeps every control', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(720, 560);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    final core = FakeCore();
    const longTitle =
        'Algumas pastas de pacotes precisam de atenção antes de continuar';
    const longDetail =
        'Verificando os arquivos do jogo e dos seus pacotes de expansão. '
        'Os arquivos alterados durante a verificação serão lidos novamente. ';
    const longReason =
        'Inicie esta instalação a partir do programa que a instalou, '
        'como o Lutris, o Heroic ou o Wine.';
    final states = <Map<String, dynamic>>[
      setup(),
      localGame(),
      localGame(
        state: 'verifying',
        status: 'Verificando',
        play: 'unavailable',
        reason: longReason,
        progress: true,
        actions: ['Pause', 'Cancel', 'ViewPacks'],
        labels: {'Pause': 'Pausar verificação', 'Cancel': 'Parar verificação'},
      ),
      localGame(
        state: 'paused',
        status: 'Pausado',
        progress: true,
        actions: ['Resume', 'Cancel', 'SetPolicy', 'ViewPacks'],
        labels: {
          'Resume': 'Retomar verificação',
          'Cancel': 'Descartar progresso',
        },
      ),
      localGame(
        state: 'installed',
        status: 'Needs attention',
        actions: ['Start', 'Verify', 'SetPolicy'],
        labels: {
          'Start': 'Iniciar a manutenção agora',
          'Verify': 'Verificar todos os arquivos',
        },
      ),
      localGame(play: 'playing', status: 'Jogando', actions: ['SetPolicy']),
      localGame(
        state: 'confirm_location',
        play: 'unavailable',
        reason: longReason,
        primary: 'ConfirmLocation',
        actions: ['ConfirmLocation', 'Discover', 'ConfirmInstall'],
        labels: {
          'ConfirmLocation': 'Manter minhas escolhas para esta pasta',
          'Discover': 'Procurar novamente',
          'ConfirmInstall': 'Usar outra pasta',
        },
      ),
    ];
    for (final base in states) {
      final s = {
        ...base,
        'title': longTitle,
        'detail': longDetail * 3,
        'next_action': longDetail,
        'collection': '12 de 106 pacotes instalados - 3 precisam de atenção',
        'path': '/home/usuario/.local/share/Steam/steamapps/common/The Sims 4 com um nome muito longo',
      };
      core.status.value = s;
      for (final scale in [1.0, 2.0]) {
        await tester.pumpWidget(
          MediaQuery(
            data: MediaQueryData(
              size: const Size(720, 560),
              textScaler: TextScaler.linear(scale),
            ),
            child: PatcherApp(core: core),
          ),
        );
        await tester.pump();
        final where = '${s['state']}/${s['play']} at ${scale}x';
        expect(tester.takeException(), isNull, reason: where);
        final lead = s['play'] == 'hidden'
            ? find.byKey(const Key('primary-action'))
            : playButton;
        expect(lead.hitTestable(), findsOneWidget, reason: 'lead in $where');
        expect(
          find.byTooltip('Settings').hitTestable(),
          findsOneWidget,
          reason: 'settings in $where',
        );
        for (final a in List<String>.from(s['actions'])) {
          final key = find.byKey(Key('action-$a'));
          if (key.evaluate().isEmpty) continue;
          await tester.ensureVisible(key);
          await tester.pump();
          expect(key.hitTestable(), findsOneWidget, reason: '$a in $where');
        }
      }
    }
    await tester.pumpWidget(const SizedBox());
  });

  testWidgets('setup leads with the folder choice and lists found copies', (
    tester,
  ) async {
    final core = await show(tester, setup());
    expect(playButton, findsNothing);
    expect(find.text('Choose game folder'), findsOneWidget);
    expect(find.byKey(const Key('action-Discover')), findsOneWidget);
    await tester.tap(find.byKey(const Key('action-Discover')));
    expect(core.commands, ['Discover']);

    core.status.value = setup(
      primary: null,
      candidates: [
        {'path': r'C:\A', 'source': 'EA app'},
        {'path': r'D:\B', 'source': 'Steam library'},
      ],
      labels: {
        'ConfirmInstall': 'Use a different folder',
        'Discover': 'Search again',
      },
    );
    await tester.pump();
    expect(find.byKey(const Key('primary-action')), findsNothing);
    expect(find.text('Use a different folder'), findsOneWidget);
    await tester.ensureVisible(find.text(r'D:\B'));
    await tester.tap(find.text(r'D:\B'));
    expect(core.commands.last, 'ConfirmInstall');
    expect(core.arguments.last, {'path': r'D:\B'});
  });

  testWidgets('a failure that cannot save offers search and folder choice', (
    tester,
  ) async {
    final core = await show(
      tester,
      setup(
        state: 'needs_attention',
        title: 'Needs attention',
        primary: 'Discover',
        actions: ['Discover', 'ConfirmInstall'],
      ),
    );
    expect(find.text('Search again'), findsOneWidget);
    expect(find.text('Choose game folder'), findsOneWidget);
    expect(find.text('Start maintenance'), findsNothing);
    await tester.tap(find.byKey(const Key('primary-action')));
    expect(core.commands, ['Discover']);
  });

  testWidgets('card actions are the engine actions with the engine labels', (
    tester,
  ) async {
    final core = await show(
      tester,
      localGame(
        actions: ['Start', 'Verify'],
        labels: {'Start': 'Start maintenance', 'Verify': 'Verify files'},
      ),
    );
    for (final label in ['Start maintenance', 'Verify files']) {
      expect(find.text(label), findsOneWidget, reason: label);
    }
    await tester.tap(find.text('Start maintenance'));
    await tester.tap(find.text('Verify files'));
    expect(core.commands, ['Start', 'Verify']);
  });

  testWidgets('progress keeps Pause beside it', (tester) async {
    final core = await show(
      tester,
      localGame(
        state: 'verifying',
        progress: true,
        play: 'unavailable',
        reason: 'Wait for maintenance to finish.',
        actions: ['Pause', 'Cancel'],
        labels: {'Pause': 'Pause', 'Cancel': 'Stop for now'},
      ),
    );
    expect(find.byKey(const Key('progress')), findsOneWidget);
    expect(find.text('3 of 10 files - 3 KB of 10 KB - 1 KB/s'), findsOneWidget);
    await tester.tap(find.text('Pause'));
    expect(core.commands, ['Pause']);
  });

  testWidgets('progress updates rebuild only the progress', (tester) async {
    final core = await show(
      tester,
      localGame(state: 'verifying', progress: true, actions: ['Pause']),
    );
    final before = homeBuilds;
    core.status.value = {
      ...core.status.value,
      'revision': 99,
      'files_done': 7,
      'detail': 'EP01/ClientFullBuild0.package',
    };
    await tester.pump();
    expect(find.textContaining('7 of 10 files'), findsOneWidget);
    expect(find.text('EP01/ClientFullBuild0.package'), findsOneWidget);
    expect(homeBuilds, before, reason: 'the main screen was not rebuilt');
  });

  testWidgets('a confirmation leads in the card when Play cannot', (
    tester,
  ) async {
    final core = await show(
      tester,
      localGame(
        state: 'confirm_location',
        play: 'unavailable',
        reason: 'Confirm your game folder first.',
        primary: 'ConfirmLocation',
        actions: ['ConfirmLocation', 'Discover', 'ConfirmInstall'],
        labels: {
          'ConfirmLocation': 'Keep my choices for this folder',
          'Discover': 'Search again',
          'ConfirmInstall': 'Use a different folder',
        },
      ),
    );
    expect(
      tester
          .widget<FilledButton>(find.byKey(const Key('action-ConfirmLocation')))
          .onPressed,
      isNotNull,
    );
    expect(find.text('Search again'), findsOneWidget);
    await tester.tap(find.text('Keep my choices for this folder'));
    expect(core.commands, ['ConfirmLocation']);
  });

  testWidgets('a staged patcher update is mentioned quietly', (tester) async {
    await show(tester, localGame(extra: {'app_update': '0.2.0'}));
    expect(find.text('Update ready - restart to apply'), findsOneWidget);
  });

  group('Settings', () {
    Future<void> open(WidgetTester tester) async {
      await tester.tap(find.byTooltip('Settings'));
      await settle(tester);
    }

    testWidgets('renders only what the engine offers', (tester) async {
      await show(tester, localGame(actions: ['ViewPacks']));
      await open(tester);
      expect(find.byKey(const Key('save-preferences')), findsNothing);
      expect(find.byKey(const Key('settings-ConfirmInstall')), findsNothing);
      expect(find.byKey(const Key('settings-Discover')), findsNothing);
      expect(find.byKey(const Key('action-CheckAppUpdate')), findsNothing);
      expect(
        find.text(
          'This copy of the patcher was not installed by its installer, so it cannot update itself.',
        ),
        findsOneWidget,
      );
      expect(find.text('Checks at login and every 6 hours.'), findsNothing);
    });

    testWidgets('offered actions run from Settings', (tester) async {
      final core = await show(
        tester,
        localGame(
          actions: [
            'SetPolicy',
            'Discover',
            'ConfirmInstall',
            'CheckAppUpdate',
          ],
          labels: {
            'SetPolicy': 'Save preferences',
            'Discover': 'Search again',
            'ConfirmInstall': 'Choose another installation',
            'CheckAppUpdate': 'Check for patcher update',
          },
        ),
      );
      await open(tester);
      expect(find.text('Choose another installation'), findsOneWidget);
      await tester.ensureVisible(find.byKey(const Key('settings-Discover')));
      await tester.tap(find.byKey(const Key('settings-Discover')));
      await settle(tester);
      expect(core.commands.last, 'Discover');
      await open(tester);
      await tester.ensureVisible(find.text('Check for patcher update'));
      await tester.tap(find.text('Check for patcher update'));
      await settle(tester);
      expect(core.commands.last, 'CheckAppUpdate');
    });

    testWidgets('language is saved with the revision; downloads stay off', (
      tester,
    ) async {
      final core = await show(tester, localGame());
      await open(tester);
      expect(find.text('Deutsch'), findsOneWidget);
      expect(find.byKey(const Key('pref-background')), findsNothing);
      expect(find.byKey(const Key('downloads-unavailable')), findsOneWidget);
      expect(
        find.text(
          'The patcher reads your game folder and never changes game files.',
        ),
        findsOneWidget,
      );
      await tester.tap(find.byKey(const Key('lang-GER_DE')));
      await tester.pump();
      await tester.ensureVisible(find.byKey(const Key('save-preferences')));
      await tester.tap(find.byKey(const Key('save-preferences')));
      await settle(tester);
      expect(core.commands.last, 'SetPolicy');
      expect(core.arguments.last, {'revision': 3, 'language': 'GER_DE'});
    });

    testWidgets('release preferences are sent together with the revision', (
      tester,
    ) async {
      final core = await show(tester, feedReady());
      await open(tester);
      expect(
        find.text(
          'Checks at login and every 6 hours. Files never change while the game is running.',
        ),
        findsOneWidget,
      );
      await tester.tap(find.byKey(const Key('pref-future')));
      await tester.tap(find.text('5 MB/s'));
      await tester.pump();
      await tester.ensureVisible(find.byKey(const Key('save-preferences')));
      await tester.tap(find.byKey(const Key('save-preferences')));
      await settle(tester);
      expect(core.commands.last, 'SetPolicy');
      expect(core.arguments.last, {
        'revision': 7,
        'preferences': {
          'future_packs': true,
          'background': false,
          'metered': false,
          'bandwidth_bytes_per_second': 5242880,
        },
      });
    });
  });

  group('Packs', () {
    testWidgets('on an own installation the list is a view, not a choice', (
      tester,
    ) async {
      await show(tester, localGame());
      expect(
        find.text('2 of 4 packs installed - 1 is missing files'),
        findsOneWidget,
      );
      await tester.tap(find.byKey(const Key('packs-action')));
      await settle(tester);
      expect(find.byKey(const Key('view-note')), findsOneWidget);
      expect(find.byType(Checkbox), findsNothing);
      expect(find.byKey(const Key('save-packs')), findsNothing);
      expect(find.textContaining('Untick'), findsNothing);
    });

    testWidgets('the list does not open unless the engine offers it', (
      tester,
    ) async {
      final core = await show(tester, localGame(actions: []));
      await tester.tap(find.byKey(const Key('packs-action')));
      await settle(tester);
      expect(find.byKey(const Key('view-note')), findsNothing);
      expect(core.commands, isEmpty);
    });

    testWidgets('saving release choices never answers an undecided pack', (
      tester,
    ) async {
      final core = await show(tester, feedReady());
      expect(find.textContaining('1 new pack to review'), findsOneWidget);
      await tester.tap(find.byKey(const Key('packs-action')));
      await settle(tester);
      await tester.tap(find.byKey(const Key('save-packs')));
      await settle(tester);
      expect(core.commands.last, 'SetPolicy');
      expect(core.arguments.last['revision'], 7);
      expect(core.arguments.last['dlc'], {'EP01': true});
      expect(core.arguments.last['deferred'], isEmpty);
    });

    testWidgets('decide later defers only untouched undecided packs', (
      tester,
    ) async {
      final core = await show(tester, feedReady());
      await tester.tap(find.byKey(const Key('packs-action')));
      await settle(tester);
      await tester.tap(find.byKey(const Key('defer-packs')));
      await settle(tester);
      expect(core.arguments.last['dlc'], {'EP01': true});
      expect(core.arguments.last['deferred'], ['SP02']);
    });

    testWidgets('the pack view filters and explains problems', (tester) async {
      tester.view.physicalSize = const Size(1200, 900);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);
      await tester.pumpWidget(
        MaterialApp(home: CollectionPage(status: localGame())),
      );
      await tester.pump();
      expect(find.byKey(const Key('stat-installed')), findsOneWidget);
      expect(find.text('1 need attention'), findsOneWidget);
      for (final id in ['EP01', 'GP05', 'SP20', 'FP01']) {
        expect(find.byKey(Key('pack-$id')), findsOneWidget, reason: id);
      }
      await tester.tap(find.byKey(const Key('state-attention')));
      await tester.pump();
      expect(find.byKey(const Key('pack-GP05')), findsOneWidget);
      expect(find.byKey(const Key('pack-EP01')), findsNothing);
      await tester.tap(find.byKey(const Key('issues-GP05')));
      await tester.pumpAndSettle();
      expect(find.text('Missing: GP05/Version.ini'), findsOneWidget);
      await tester.tap(find.text('Close'));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('state-all')));
      await tester.tap(find.byKey(const Key('kind-kit')));
      await tester.pump();
      expect(find.byKey(const Key('pack-SP20')), findsOneWidget);
      expect(find.byKey(const Key('pack-GP05')), findsNothing);
      await tester.tap(find.byKey(const Key('kind-all')));
      await tester.enterText(find.byKey(const Key('pack-search')), 'holiday');
      await tester.pump();
      expect(find.byKey(const Key('pack-FP01')), findsOneWidget);
      expect(find.byKey(const Key('pack-EP01')), findsNothing);
      expect(tester.takeException(), isNull);
    });
  });
}
