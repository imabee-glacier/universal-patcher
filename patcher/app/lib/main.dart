import 'package:flutter/material.dart';
import 'package:flutter/scheduler.dart';
import 'package:file_selector/file_selector.dart';

import 'collection.dart';
import 'core.dart';

const ink = Color(0xff233b38);
const teal = Color(0xff176b5b);
const paper = Color(0xfff5f3ed);
const muted = Color(0xff697772);
const line = Color(0xffdedfd6);

void main() {
  WidgetsFlutterBinding.ensureInitialized();
  try {
    runApp(PatcherApp(core: NativeCore()));
  } catch (error) {
    runApp(
      MaterialApp(
        home: Scaffold(
          body: Center(
            child: Padding(
              padding: const EdgeInsets.all(32),
              child: Text(
                'The maintenance engine could not start.\nReinstall the patcher to restore its native library.\n\n$error',
              ),
            ),
          ),
        ),
      ),
    );
  }
}

class PatcherApp extends StatelessWidget {
  const PatcherApp({super.key, required this.core});
  final Core core;
  @override
  Widget build(BuildContext context) => MaterialApp(
    title: 'Patcher',
    debugShowCheckedModeBanner: false,
    theme: ThemeData(
      useMaterial3: true,
      scaffoldBackgroundColor: paper,
      colorScheme: ColorScheme.fromSeed(seedColor: teal, surface: paper),
      textTheme: const TextTheme(
        bodyMedium: TextStyle(color: ink, fontSize: 15, height: 1.5),
        bodySmall: TextStyle(color: muted, fontSize: 12, height: 1.5),
      ),
      filledButtonTheme: FilledButtonThemeData(
        style: FilledButton.styleFrom(
          backgroundColor: teal,
          foregroundColor: Colors.white,
          minimumSize: const Size(120, 48),
          shape: RoundedRectangleBorder(
            borderRadius: BorderRadius.circular(14),
          ),
        ),
      ),
      outlinedButtonTheme: OutlinedButtonThemeData(
        style: OutlinedButton.styleFrom(
          foregroundColor: ink,
          minimumSize: const Size(100, 46),
          side: const BorderSide(color: line),
          shape: RoundedRectangleBorder(
            borderRadius: BorderRadius.circular(14),
          ),
        ),
      ),
      dividerColor: line,
      snackBarTheme: const SnackBarThemeData(backgroundColor: ink),
    ),
    home: Home(core: core),
  );
}

class Home extends StatefulWidget {
  const Home({super.key, required this.core});
  final Core core;
  @override
  State<Home> createState() => _HomeState();
}

class _HomeState extends State<Home> with SingleTickerProviderStateMixin {
  late final Ticker _ticker;
  bool details = false;
  @override
  void initState() {
    super.initState();
    widget.core.tick();
    _ticker = createTicker((_) => widget.core.tick())..start();
  }

  @override
  void dispose() {
    _ticker.dispose();
    super.dispose();
  }

  void send(String command, [Map<String, dynamic> args = const {}]) {
    final result = widget.core.command(command, args);
    if (result['accepted'] != true) {
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(
          content: Text(
            result['error']?.toString() ?? 'That action is unavailable.',
          ),
        ),
      );
    }
  }

  Future<void> chooseFolder() async {
    final path = await getDirectoryPath(
      confirmButtonText: 'Use this installation',
    );
    if (path != null && mounted) {
      send('ConfirmInstall', {'path': path});
    }
  }

  void openCollection(Map<String, dynamic> s) {
    Navigator.of(context).push(
      MaterialPageRoute<void>(
        builder: (_) => CollectionPage(
          status: s,
          save: (dlc) {
            final result = widget.core.command('SetPolicy', {
              'revision': s['policy_revision'],
              'dlc': dlc,
            });
            return result['accepted'] == true
                ? null
                : result['error']?.toString() ?? 'Your choices were not saved.';
          },
        ),
      ),
    );
  }

  @override
  Widget build(BuildContext context) => Scaffold(
    body: SafeArea(
      child: ValueListenableBuilder<Map<String, dynamic>>(
        valueListenable: widget.core.status,
        builder: (context, s, _) {
          final actions = List<String>.from(s['actions'] ?? []);
          // A game is in use, or found installations are listed to choose from.
          final located =
              s['path'] != null || (s['candidates'] as List? ?? []).isNotEmpty;
          final ready = s['ready'] == true;
          final fixture = s['fixture'] == true;
          final packs = List<Map<String, dynamic>>.from(s['packs'] ?? []);
          final active = actions.contains('Pause');
          // The user's own installation, read in place (no release feed).
          final local = s['state'] == 'installed';
          final review = packs.any(
            (p) =>
                p['status'] != 'not_installed' &&
                p['status'] != '' &&
                p['answered'] != true &&
                p['deferred'] != true,
          );
          return LayoutBuilder(
            builder: (context, constraints) => SingleChildScrollView(
              child: ConstrainedBox(
                constraints: BoxConstraints(minHeight: constraints.maxHeight),
                child: Padding(
                  padding: EdgeInsets.symmetric(
                    horizontal: constraints.maxWidth > 900 ? 64 : 32,
                    vertical: 28,
                  ),
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.stretch,
                    children: [
                      Row(
                        children: [
                          Container(
                            width: 40,
                            height: 40,
                            decoration: BoxDecoration(
                              color: teal,
                              borderRadius: BorderRadius.circular(13),
                            ),
                            child: const Icon(
                              Icons.spa_outlined,
                              color: Colors.white,
                              size: 22,
                            ),
                          ),
                          const SizedBox(width: 12),
                          const Expanded(
                            child: Column(
                              crossAxisAlignment: CrossAxisAlignment.start,
                              children: [
                                Text(
                                  'PATCHER',
                                  style: TextStyle(
                                    color: ink,
                                    fontSize: 13,
                                    fontWeight: FontWeight.w800,
                                    letterSpacing: 2,
                                  ),
                                ),
                                Text(
                                  'A little care for your game.',
                                  style: TextStyle(color: muted, fontSize: 12),
                                ),
                              ],
                            ),
                          ),
                          if (fixture)
                            const BadgeLabel(text: 'TEST INSTALLATION'),
                          const SizedBox(width: 12),
                          IconButton(
                            tooltip: 'Settings',
                            onPressed: () => settings(s),
                            icon: const Icon(Icons.tune_rounded, color: ink),
                          ),
                        ],
                      ),
                      const SizedBox(height: 36),
                      Row(
                        children: [
                          const Expanded(
                            child: Text(
                              'THE SIMS 4',
                              style: TextStyle(
                                color: muted,
                                fontSize: 11,
                                fontWeight: FontWeight.w700,
                                letterSpacing: 2.3,
                              ),
                            ),
                          ),
                          Icon(
                            ready
                                ? Icons.check_circle_outline_rounded
                                : Icons.circle_outlined,
                            size: 15,
                            color: ready ? teal : muted,
                          ),
                          const SizedBox(width: 6),
                          Text(
                            ready
                                ? 'Verified'
                                : stateLabel(
                                    s['state']?.toString() ?? 'checking',
                                  ),
                            style: const TextStyle(color: muted, fontSize: 12),
                          ),
                        ],
                      ),
                      const SizedBox(height: 20),
                      Container(
                        padding: const EdgeInsets.all(28),
                        decoration: BoxDecoration(
                          color: const Color(0xffe8eee6),
                          borderRadius: BorderRadius.circular(24),
                          border: Border.all(color: const Color(0xffdce5da)),
                        ),
                        child: Column(
                          crossAxisAlignment: CrossAxisAlignment.start,
                          children: [
                            Row(
                              crossAxisAlignment: CrossAxisAlignment.start,
                              children: [
                                Expanded(
                                  child: Text(
                                    s['title']?.toString() ??
                                        'Finding your game',
                                    style: const TextStyle(
                                      color: ink,
                                      fontSize: 38,
                                      fontWeight: FontWeight.w500,
                                      letterSpacing: -1.5,
                                      height: 1.1,
                                    ),
                                  ),
                                ),
                                const SizedBox(width: 16),
                                Icon(
                                  ready
                                      ? Icons.task_alt_rounded
                                      : active
                                      ? Icons.autorenew_rounded
                                      : Icons.games_outlined,
                                  size: 42,
                                  color: teal,
                                ),
                              ],
                            ),
                            const SizedBox(height: 16),
                            Text(
                              s['detail']?.toString() ?? 'Checking saved locations and Steam libraries.',
                              style: const TextStyle(
                                color: muted,
                                fontSize: 14,
                                height: 1.6,
                              ),
                            ),
                            if (active || working.contains(s['state'])) ...[
                              const SizedBox(height: 24),
                              ProgressView(core: widget.core),
                            ],
                            const SizedBox(height: 24),
                            Wrap(
                              spacing: 12,
                              runSpacing: 12,
                              crossAxisAlignment: WrapCrossAlignment.center,
                              children: [
                                if (local && review)
                                  FilledButton.icon(
                                    key: const Key('primary-action'),
                                    onPressed: () => openCollection(s),
                                    icon: const Icon(Icons.grid_view_rounded),
                                    label: const Text('Review packs'),
                                  )
                                else if (local && actions.contains('Launch'))
                                  FilledButton.icon(
                                    key: const Key('primary-action'),
                                    onPressed: () => send('Launch'),
                                    icon: const Icon(Icons.play_arrow_rounded),
                                    label: const Text('Play'),
                                  )
                                else if (actions.contains('Resume'))
                                  FilledButton.icon(
                                    key: const Key('primary-action'),
                                    onPressed: () => send('Resume'),
                                    icon: const Icon(Icons.play_arrow_rounded),
                                    label: const Text('Resume maintenance'),
                                  )
                                else if (actions.contains('Start'))
                                  FilledButton.icon(
                                    key: const Key('primary-action'),
                                    onPressed: () => send('Start'),
                                    icon: const Icon(
                                      Icons.arrow_forward_rounded,
                                    ),
                                    label: const Text('Start maintenance'),
                                  )
                                else if (active)
                                  FilledButton.icon(
                                    key: const Key('primary-action'),
                                    onPressed: () => send('Pause'),
                                    icon: const Icon(Icons.pause_rounded),
                                    label: const Text('Pause'),
                                  )
                                else if (ready)
                                  FilledButton.icon(
                                    key: const Key('primary-action'),
                                    onPressed: actions.contains('Launch')
                                        ? () => send('Launch')
                                        : null,
                                    icon: const Icon(Icons.play_arrow_rounded),
                                    label: const Text('Play'),
                                  )
                                // The folder picker leads only when no installation was
                                // found; otherwise the found game or the list is the choice.
                                else if (actions.contains('ConfirmInstall') &&
                                    !located)
                                  FilledButton.icon(
                                    key: const Key('primary-action'),
                                    onPressed: chooseFolder,
                                    icon: const Icon(
                                      Icons.folder_open_outlined,
                                    ),
                                    label: const Text('Choose game folder'),
                                  ),
                                if (actions.contains('ConfirmInstall') &&
                                    located)
                                  TextButton.icon(
                                    key: const Key('other-folder'),
                                    onPressed: chooseFolder,
                                    icon: const Icon(
                                      Icons.folder_open_outlined,
                                      size: 18,
                                    ),
                                    label: const Text('Use a different folder'),
                                  ),
                                if (local &&
                                    review &&
                                    actions.contains('Launch'))
                                  OutlinedButton.icon(
                                    key: const Key('play-action'),
                                    onPressed: () => send('Launch'),
                                    icon: const Icon(
                                      Icons.play_arrow_rounded,
                                      size: 18,
                                    ),
                                    label: const Text('Play'),
                                  ),
                                if (actions.contains('Verify'))
                                  OutlinedButton.icon(
                                    key: const Key('verify-action'),
                                    onPressed: () => send('Verify'),
                                    icon: const Icon(
                                      Icons.verified_outlined,
                                      size: 18,
                                    ),
                                    label: Text(
                                      local ? 'Check files' : 'Verify files',
                                    ),
                                  ),
                                if (actions.contains('AcceptCheck'))
                                  TextButton.icon(
                                    key: const Key('accept-check'),
                                    onPressed: () => send('AcceptCheck'),
                                    icon: const Icon(
                                      Icons.done_all_rounded,
                                      size: 18,
                                    ),
                                    label: const Text('Accept these changes'),
                                  ),
                                if (actions.contains('Cancel'))
                                  TextButton(
                                    key: const Key('stop-action'),
                                    onPressed: () => send('Cancel'),
                                    child: Text(
                                      s['state'] == 'verifying'
                                          ? 'Stop check'
                                          : 'Stop for now',
                                    ),
                                  ),
                              ],
                            ),
                            const SizedBox(height: 12),
                            Text(
                              s['next_action']?.toString() ?? '',
                              style: const TextStyle(
                                color: muted,
                                fontSize: 12,
                              ),
                            ),
                          ],
                        ),
                      ),
                      if (s['path'] != null) ...[
                        const SizedBox(height: 16),
                        Row(
                          children: [
                            const Icon(
                              Icons.folder_outlined,
                              size: 16,
                              color: muted,
                            ),
                            const SizedBox(width: 8),
                            Expanded(
                              child: Text(
                                s['path'],
                                maxLines: 1,
                                overflow: TextOverflow.ellipsis,
                                style: const TextStyle(
                                  color: muted,
                                  fontSize: 12,
                                ),
                              ),
                            ),
                          ],
                        ),
                      ],
                      if ((s['candidates'] as List? ?? []).isNotEmpty &&
                          s['path'] == null) ...[
                        const SizedBox(height: 12),
                        for (final candidate in s['candidates'])
                          ListTile(
                            leading: const Icon(Icons.folder_outlined),
                            title: Text(candidate['path']),
                            subtitle: Text(candidate['source']),
                            trailing: const Icon(Icons.arrow_forward),
                            onTap: () => send('ConfirmInstall', {
                              'path': candidate['path'],
                            }),
                          ),
                      ],
                      const SizedBox(height: 28),
                      Row(
                        crossAxisAlignment: CrossAxisAlignment.start,
                        children: [
                          Expanded(
                            child: InkWell(
                              key: const Key('packs-action'),
                              borderRadius: BorderRadius.circular(12),
                              onTap: packs.isNotEmpty
                                  ? () =>
                                        local ? openCollection(s) : packSheet(s)
                                  : null,
                              child: Padding(
                                padding: const EdgeInsets.symmetric(
                                  vertical: 8,
                                ),
                                child: Row(
                                  children: [
                                    const Icon(
                                      Icons.grid_view_rounded,
                                      color: teal,
                                      size: 21,
                                    ),
                                    const SizedBox(width: 12),
                                    Expanded(
                                      child: Column(
                                        crossAxisAlignment:
                                            CrossAxisAlignment.start,
                                        children: [
                                          const Text(
                                            'Your collection',
                                            style: TextStyle(
                                              color: ink,
                                              fontWeight: FontWeight.w600,
                                            ),
                                          ),
                                          Text(
                                            packs.isEmpty
                                                ? 'Shown once your game is found'
                                                : collectionSummary(packs),
                                            style: const TextStyle(
                                              color: muted,
                                              fontSize: 12,
                                            ),
                                          ),
                                        ],
                                      ),
                                    ),
                                    if (packs.isNotEmpty)
                                      const Icon(
                                        Icons.chevron_right,
                                        color: muted,
                                      ),
                                  ],
                                ),
                              ),
                            ),
                          ),
                          const SizedBox(width: 24),
                          Expanded(
                            child: Padding(
                              padding: const EdgeInsets.symmetric(vertical: 8),
                              child: Row(
                                crossAxisAlignment: CrossAxisAlignment.start,
                                children: [
                                  const Icon(
                                    Icons.cloud_outlined,
                                    color: muted,
                                    size: 21,
                                  ),
                                  const SizedBox(width: 12),
                                  Expanded(
                                    child: Column(
                                      crossAxisAlignment:
                                          CrossAxisAlignment.start,
                                      children: [
                                        const Text(
                                          'Online access',
                                          style: TextStyle(
                                            color: ink,
                                            fontWeight: FontWeight.w600,
                                          ),
                                        ),
                                        Text(
                                          s['online']?.toString() ??
                                              'Checking availability',
                                          style: const TextStyle(
                                            color: muted,
                                            fontSize: 12,
                                          ),
                                        ),
                                      ],
                                    ),
                                  ),
                                ],
                              ),
                            ),
                          ),
                        ],
                      ),
                      const SizedBox(height: 20),
                      const Divider(),
                      Align(
                        alignment: Alignment.centerLeft,
                        child: TextButton.icon(
                          onPressed: () => setState(() => details = !details),
                          icon: Icon(
                            details ? Icons.expand_less : Icons.expand_more,
                            size: 18,
                          ),
                          label: const Text('Details'),
                        ),
                      ),
                      if (details)
                        SelectionArea(
                          child: Padding(
                            padding: const EdgeInsets.all(12),
                            child: Text(
                              [
                                if (s['error'] != null) s['error'].toString(),
                                if (fixture) 'This is a signed test release. No game content is included.',
                                ...List<dynamic>.from(s['details'] ?? [])
                                    .map((line) => line.toString()),
                                if ((s['checked'] as List?)?.isNotEmpty ??
                                    false) ...[
                                  'Locations checked:',
                                  ...List<dynamic>.from(s['checked'])
                                      .map((p) => p.toString()),
                                ],
                              ].join('\n'),
                              style: const TextStyle(
                                color: muted,
                                fontSize: 12,
                              ),
                            ),
                          ),
                        ),
                    ],
                  ),
                ),
              ),
            ),
          );
        },
      ),
    ),
  );
  void settings(Map<String, dynamic> s) {
    final saved = Map<String, dynamic>.from(s['preferences'] ?? const {});
    final languages = List<String>.from(s['languages'] ?? const []);
    String? language = s['language'] as String?;
    // Download settings only mean something with a release feed.
    final downloads = s['state'] != 'installed';
    final prefs = <String, dynamic>{
      'future_packs': saved['future_packs'] == true,
      'background': saved['background'] == true,
      'metered': saved['metered'] == true,
      'bandwidth_bytes_per_second': saved['bandwidth_bytes_per_second'] ?? 0,
    };
    showModalBottomSheet<void>(
      context: context,
      showDragHandle: true,
      isScrollControlled: true,
      builder: (sheetContext) => StatefulBuilder(
        builder: (sheetContext, update) => SingleChildScrollView(
          padding: const EdgeInsets.fromLTRB(32, 8, 32, 32),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              const Text(
                'Your preferences',
                style: TextStyle(fontSize: 26, color: ink),
              ),
              const SizedBox(height: 12),
              if (languages.isNotEmpty) ...[
                const Text('Game language', style: TextStyle(color: ink)),
                const SizedBox(height: 8),
                Wrap(
                  spacing: 8,
                  runSpacing: 8,
                  children: [
                    for (final code in languages)
                      ChoiceChip(
                        key: Key('lang-$code'),
                        label: Text(languageName(code)),
                        selected: language == code,
                        onSelected: (_) => update(() => language = code),
                      ),
                  ],
                ),
                const SizedBox(height: 6),
                const Text(
                  'Languages installed with your game. The choice is saved for when language switching is validated; your game is not changed yet.',
                  style: TextStyle(color: muted, fontSize: 12),
                ),
                const SizedBox(height: 16),
              ],
              if (!downloads)
                const ListTile(
                  key: Key('downloads-unavailable'),
                  contentPadding: EdgeInsets.zero,
                  leading: Icon(Icons.cloud_off_outlined),
                  title: Text('Downloads and background maintenance'),
                  subtitle: Text(
                    'Available once game releases are published. Until then the patcher checks your own files only.',
                  ),
                ),
              if (downloads && s['background_supported'] != false)
                SwitchListTile(
                  key: const Key('pref-background'),
                  contentPadding: EdgeInsets.zero,
                  secondary: const Icon(Icons.schedule_outlined),
                  title: const Text('Maintain in the background'),
                  subtitle: const Text(
                    'Checks at login and every 6 hours. Files never change while the game is running.',
                  ),
                  value: prefs['background'] as bool,
                  onChanged: (v) => update(() => prefs['background'] = v),
                ),
              if (downloads) ...[
                SwitchListTile(
                  key: const Key('pref-future'),
                  contentPadding: EdgeInsets.zero,
                  secondary: const Icon(Icons.new_releases_outlined),
                  title: const Text('Include future packs automatically'),
                  subtitle: const Text(
                    'New packs download without asking. This uses more disk space and bandwidth.',
                  ),
                  value: prefs['future_packs'] as bool,
                  onChanged: (v) => update(() => prefs['future_packs'] = v),
                ),
                SwitchListTile(
                  key: const Key('pref-metered'),
                  contentPadding: EdgeInsets.zero,
                  secondary: const Icon(Icons.signal_cellular_alt_outlined),
                  title: const Text('Allow metered connections'),
                  subtitle: const Text(
                    'Background downloads otherwise wait for an unmetered network.',
                  ),
                  value: prefs['metered'] as bool,
                  onChanged: (v) => update(() => prefs['metered'] = v),
                ),
                const SizedBox(height: 8),
                const Text(
                  'Download speed limit',
                  style: TextStyle(color: ink),
                ),
                const SizedBox(height: 8),
                Wrap(
                  spacing: 8,
                  runSpacing: 8,
                  children: [
                    for (final (label, value) in bandwidthChoices)
                      ChoiceChip(
                        label: Text(label),
                        selected: prefs['bandwidth_bytes_per_second'] == value,
                        onSelected: (_) => update(
                          () => prefs['bandwidth_bytes_per_second'] = value,
                        ),
                      ),
                  ],
                ),
              ],
              const SizedBox(height: 16),
              const ListTile(
                contentPadding: EdgeInsets.zero,
                leading: Icon(Icons.person_outline),
                title: Text('Online sign-in'),
                subtitle: Text(
                  'Unavailable until the sign-in handoff is verified.',
                ),
              ),
              const SizedBox(height: 12),
              Wrap(
                spacing: 12,
                runSpacing: 12,
                children: [
                  FilledButton(
                    key: const Key('save-preferences'),
                    onPressed: () {
                      Navigator.pop(sheetContext);
                      send('SetPolicy', {
                        'revision': s['policy_revision'],
                        if (downloads) 'preferences': prefs,
                        if (languages.isNotEmpty) 'language': language,
                      });
                    },
                    child: const Text('Save preferences'),
                  ),
                  OutlinedButton.icon(
                    onPressed: () {
                      Navigator.pop(sheetContext);
                      chooseFolder();
                    },
                    icon: const Icon(Icons.folder_open_outlined),
                    label: const Text('Choose another installation'),
                  ),
                ],
              ),
              const SizedBox(height: 16),
              Text(
                s['fixture'] == true
                    ? 'Test mode uses an independent local signing key and synthetic content.'
                    : 'The patcher reads your game folder; it does not change game files.',
                style: const TextStyle(color: muted, fontSize: 12),
              ),
            ],
          ),
        ),
      ),
    );
  }

  void packSheet(Map<String, dynamic> s) {
    final packs = List<Map<String, dynamic>>.from(s['packs']);
    // Only answered packs start with a decision; new packs stay unanswered until touched.
    final selections = <String, bool>{
      for (final p in packs)
        if (p['answered'] == true) p['id']: p['selected'] == true,
    };
    final deferred = <String>{
      for (final p in packs)
        if (p['deferred'] == true) p['id'] as String,
    };
    bool isNew(Map<String, dynamic> p) =>
        p['answered'] != true && p['deferred'] != true;
    packs.sort((a, b) => (isNew(a) ? 0 : 1).compareTo(isNew(b) ? 0 : 1));
    String query = '';
    showModalBottomSheet<void>(
      context: context,
      showDragHandle: true,
      isScrollControlled: true,
      builder: (sheetContext) => StatefulBuilder(
        builder: (sheetContext, update) => SizedBox(
          height: MediaQuery.sizeOf(sheetContext).height * .8,
          child: Padding(
            padding: const EdgeInsets.fromLTRB(32, 8, 32, 24),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                const Text(
                  'Make it your collection',
                  style: TextStyle(
                    color: ink,
                    fontSize: 28,
                    letterSpacing: -.7,
                  ),
                ),
                const SizedBox(height: 8),
                const Text(
                  'Choose once. The patcher remembers.',
                  style: TextStyle(color: muted),
                ),
                const SizedBox(height: 20),
                TextField(
                  decoration: const InputDecoration(
                    prefixIcon: Icon(Icons.search),
                    hintText: 'Find a pack',
                    border: OutlineInputBorder(),
                  ),
                  onChanged: (text) => update(() => query = text.toLowerCase()),
                ),
                const SizedBox(height: 12),
                Expanded(
                  child: ListView(
                    children: [
                      for (final p in packs.where(
                        (p) =>
                            p['name'].toString().toLowerCase().contains(query),
                      ))
                        CheckboxListTile(
                          key: Key('pack-${p['id']}'),
                          contentPadding: EdgeInsets.zero,
                          tristate: !selections.containsKey(p['id']),
                          title: Text(p['name']),
                          subtitle: Text(
                            [
                              p['id'],
                              '${formatBytes(p['bytes'] ?? 0)} download',
                              if (!selections.containsKey(p['id']))
                                deferred.contains(p['id'])
                                    ? 'Decided later'
                                    : 'New: not decided',
                            ].join(' - '),
                          ),
                          value: selections[p['id']],
                          onChanged: (v) => update(() {
                            selections[p['id']] = v ?? false;
                            deferred.remove(p['id']);
                          }),
                        ),
                    ],
                  ),
                ),
                const SizedBox(height: 12),
                Wrap(
                  spacing: 12,
                  runSpacing: 12,
                  children: [
                    FilledButton(
                      key: const Key('save-packs'),
                      onPressed: () {
                        Navigator.pop(sheetContext);
                        send('SetPolicy', {
                          'revision': s['policy_revision'],
                          'dlc': selections,
                          'deferred': deferred.toList(),
                        });
                      },
                      child: const Text('Save selections'),
                    ),
                    if (packs.any(
                      (p) => isNew(p) && !selections.containsKey(p['id']),
                    ))
                      TextButton(
                        key: const Key('defer-packs'),
                        onPressed: () {
                          Navigator.pop(sheetContext);
                          send('SetPolicy', {
                            'revision': s['policy_revision'],
                            'dlc': selections,
                            'deferred': {
                              ...deferred,
                              for (final p in packs)
                                if (isNew(p) &&
                                    !selections.containsKey(p['id']))
                                  p['id'] as String,
                            }.toList(),
                          });
                        },
                        child: const Text('Decide new packs later'),
                      ),
                  ],
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

class ProgressView extends StatelessWidget {
  const ProgressView({super.key, required this.core});
  final Core core;
  @override
  Widget build(BuildContext context) =>
      ValueListenableBuilder<Map<String, dynamic>>(
        valueListenable: core.progress,
        builder: (context, p, _) {
          final done = (p['bytes_done'] as num? ?? 0).toDouble(),
              total = (p['bytes_total'] as num? ?? 0).toDouble();
          return Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              LinearProgressIndicator(
                value: total > 0 ? (done / total).clamp(0, 1) : null,
                minHeight: 5,
                borderRadius: BorderRadius.circular(8),
                backgroundColor: const Color(0xffd6e1d5),
                color: teal,
              ),
              const SizedBox(height: 10),
              Text(
                [
                  if ((p['files_total'] as num? ?? 0) > 0)
                    '${p['files_done']} of ${p['files_total']} files',
                  if (total > 0)
                    '${formatBytes(done)} of ${formatBytes(total)}',
                  if (total > 0) '${formatBytes(p['bytes_per_second'] ?? 0)}/s',
                ].join(' · ').ifEmpty('Checking local files'),
                style: const TextStyle(color: muted, fontSize: 12),
              ),
            ],
          );
        },
      );
}

class BadgeLabel extends StatelessWidget {
  const BadgeLabel({super.key, required this.text});
  final String text;
  @override
  Widget build(BuildContext context) => Container(
    padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 6),
    decoration: BoxDecoration(
      border: Border.all(color: line),
      borderRadius: BorderRadius.circular(20),
    ),
    child: Text(
      text,
      style: const TextStyle(
        color: muted,
        fontSize: 9,
        letterSpacing: 1,
        fontWeight: FontWeight.w600,
      ),
    ),
  );
}

String stateLabel(String state) => switch (state) {
  'needs_attention' => 'Needs attention',
  'waiting' => 'Waiting for game to close',
  'ready' => 'Ready',
  'setup' => 'Setup',
  'installed' => 'Installed',
  'checking' => 'Checking',
  'downloading' => 'Downloading',
  'applying' => 'Applying',
  'verifying' => 'Verifying',
  'paused' => 'Paused',
  'cancelled' => 'Stopped',
  'planned' => 'Selections saved',
  _ => state,
};
String formatBytes(num value) {
  if (value >= 1073741824) {
    return '${(value / 1073741824).toStringAsFixed(1)} GB';
  }
  if (value >= 1048576) {
    return '${(value / 1048576).toStringAsFixed(1)} MB';
  }
  if (value >= 1024) {
    return '${(value / 1024).toStringAsFixed(0)} KB';
  }
  return '${value.toInt()} B';
}

const bandwidthChoices = [
  ('Unlimited', 0),
  ('1 MB/s', 1048576),
  ('5 MB/s', 5242880),
  ('20 MB/s', 20971520),
];

String collectionSummary(List<Map<String, dynamic>> packs) {
  if (packs.any((p) => (p['status'] ?? '') != '')) {
    final installed = packs.where((p) => p['status'] != 'not_installed').length;
    final flagged = packs
        .where((p) => p['status'] == 'incomplete' || p['status'] == 'changed')
        .length;
    return [
      '$installed of ${packs.length} installed',
      if (flagged > 0) '$flagged ${flagged == 1 ? 'needs' : 'need'} attention',
    ].join(' · ');
  }
  final selected = packs.where((p) => p['selected'] == true).length;
  final fresh = packs
      .where((p) => p['answered'] != true && p['deferred'] != true)
      .length;
  final summary = '$selected of ${packs.length} packs selected';
  if (fresh == 0) return summary;
  return '$summary - $fresh new ${fresh == 1 ? 'pack' : 'packs'} to review';
}

/// States with work in progress, where progress is shown.
const working = {
  'verifying',
  'downloading',
  'applying',
  'checking',
  'paused',
  'waiting',
};

String languageName(String code) => switch (code) {
  'ENG_US' => 'English',
  'GER_DE' => 'Deutsch',
  'FRE_FR' => 'Français',
  'SPA_ES' => 'Español',
  'ITA_IT' => 'Italiano',
  'POR_BR' => 'Português (Brasil)',
  'DUT_NL' => 'Nederlands',
  'POL_PL' => 'Polski',
  'RUS_RU' => 'Русский',
  'CZE_CZ' => 'Čeština',
  'DAN_DK' => 'Dansk',
  'FIN_FI' => 'Suomi',
  'NOR_NO' => 'Norsk',
  'SWE_SE' => 'Svenska',
  'JPN_JP' => '日本語',
  'KOR_KR' => '한국어',
  'CHS_CN' => '简体中文',
  'CHT_CN' => '繁體中文',
  _ => code,
};

extension on String {
  String ifEmpty(String fallback) => isEmpty ? fallback : this;
}
