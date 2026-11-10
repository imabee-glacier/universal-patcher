import 'package:flutter/material.dart';

import 'main.dart' show formatBytes;
import 'tokens.dart';

/// Colours for each kind of pack: one hue per kind, used as a band and tag.
const kindColors = {
  'expansion': Color(0xff176b5b),
  'game': Color(0xff5b4a8b),
  'stuff': Color(0xffb7791f),
  'kit': Color(0xffb04a6a),
  'free': muted,
};
const kindNames = {
  'expansion': 'Expansion',
  'game': 'Game pack',
  'stuff': 'Stuff pack',
  'kit': 'Kit',
  'free': 'Free',
};
const attention = warning;

bool needsAttention(Map<String, dynamic> p) => p['status'] == 'incomplete';
bool isInstalled(Map<String, dynamic> p) =>
    p['status'] != 'not_installed' && p['status'] != '';

/// Every pack the game knows, with what is on this computer: installed, incomplete or not
/// installed. A view only: without published releases there is
/// nothing a choice here could download or remove, so none is offered.
class CollectionPage extends StatefulWidget {
  const CollectionPage({super.key, required this.status});

  final Map<String, dynamic> status;

  @override
  State<CollectionPage> createState() => _CollectionPageState();
}

class _CollectionPageState extends State<CollectionPage> {
  late final List<Map<String, dynamic>> packs;
  String query = '';
  String kind = 'all';
  String state = 'all';

  @override
  void initState() {
    super.initState();
    packs = List<Map<String, dynamic>>.from(widget.status['packs'] ?? []);
  }

  List<Map<String, dynamic>> get shown => packs.where((p) {
    final text = '${p['name']} ${p['id']}'.toLowerCase();
    if (query.isNotEmpty && !text.contains(query)) return false;
    if (kind != 'all' && p['kind'] != kind) return false;
    return switch (state) {
      'installed' => isInstalled(p),
      'not_installed' => p['status'] == 'not_installed',
      'attention' => needsAttention(p),
      _ => true,
    };
  }).toList();

  void issues(Map<String, dynamic> p) => showDialog<void>(
    context: context,
    builder: (context) => AlertDialog(
      title: Text(p['name']),
      content: SizedBox(
        width: 520,
        child: SelectionArea(
          child: ListView(
            shrinkWrap: true,
            children: [
              for (final issue in List<dynamic>.from(p['issues'] ?? []))
                Padding(
                  padding: const EdgeInsets.symmetric(vertical: 2),
                  child: Text(
                    issue.toString(),
                    style: const TextStyle(fontSize: 12, color: muted),
                  ),
                ),
            ],
          ),
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.pop(context),
          child: const Text('Close'),
        ),
      ],
    ),
  );

  @override
  Widget build(BuildContext context) {
    // Counts for the chips are tallies of the statuses the engine sent; nothing is decided here.
    final installed = packs.where(isInstalled).length;
    final flagged = packs.where(needsAttention).length;
    final onDisk = packs.fold<num>(
      0,
      (sum, p) => sum + (p['installed_bytes'] as num? ?? 0),
    );
    final list = shown;
    return Scaffold(
      backgroundColor: paper,
      appBar: AppBar(
        backgroundColor: paper,
        surfaceTintColor: paper,
        title: const Text('Your packs', style: TextStyle(color: ink)),
      ),
      body: LayoutBuilder(
        builder: (context, box) {
          final columns = (box.maxWidth / 260).floor().clamp(1, 5);
          return CustomScrollView(
            slivers: [
              SliverPadding(
                padding: const EdgeInsets.fromLTRB(24, 4, 24, 0),
                sliver: SliverList.list(
                  children: [
                    Wrap(
                      spacing: 8,
                      runSpacing: 8,
                      children: [
                        _Stat(
                          key: const Key('stat-installed'),
                          icon: Icons.check_circle_outline_rounded,
                          color: teal,
                          text: '$installed installed',
                        ),
                        _Stat(
                          icon: Icons.radio_button_unchecked,
                          color: muted,
                          text: '${packs.length - installed} not installed',
                        ),
                        if (flagged > 0)
                          _Stat(
                            key: const Key('stat-attention'),
                            icon: Icons.error_outline_rounded,
                            color: attention,
                            text: '$flagged need attention',
                          ),
                        if (onDisk > 0)
                          _Stat(
                            icon: Icons.storage_rounded,
                            color: muted,
                            text: '${formatBytes(onDisk)} on disk',
                          ),
                      ],
                    ),
                    const SizedBox(height: 16),
                    Container(
                      key: const Key('view-note'),
                      padding: const EdgeInsets.all(16),
                      decoration: BoxDecoration(
                        color: mint,
                        borderRadius: BorderRadius.circular(14),
                      ),
                      child: const Row(
                        children: [
                          Icon(Icons.info_outline_rounded, color: teal),
                          SizedBox(width: 16),
                          Expanded(
                            child: Text(
                              'What is installed on this computer. The patcher does not '
                              'download or remove packs.',
                              style: TextStyle(color: ink),
                            ),
                          ),
                        ],
                      ),
                    ),
                    const SizedBox(height: 16),
                    TextField(
                      key: const Key('pack-search'),
                      decoration: InputDecoration(
                        prefixIcon: const Icon(Icons.search),
                        hintText: 'Find a pack by name or id',
                        filled: true,
                        fillColor: surface,
                        border: OutlineInputBorder(
                          borderRadius: BorderRadius.circular(14),
                          borderSide: const BorderSide(color: line),
                        ),
                      ),
                      onChanged: (t) => setState(() => query = t.toLowerCase()),
                    ),
                    const SizedBox(height: 16),
                    Wrap(
                      spacing: 8,
                      runSpacing: 8,
                      children: [
                        for (final (value, label) in [
                          ('all', 'All kinds'),
                          ('expansion', 'Expansions'),
                          ('game', 'Game packs'),
                          ('stuff', 'Stuff packs'),
                          ('kit', 'Kits'),
                          ('free', 'Free'),
                        ])
                          ChoiceChip(
                            key: Key('kind-$value'),
                            label: Text(label),
                            selected: kind == value,
                            onSelected: (_) => setState(() => kind = value),
                          ),
                      ],
                    ),
                    const SizedBox(height: 8),
                    Wrap(
                      spacing: 8,
                      runSpacing: 8,
                      children: [
                        for (final (value, label) in [
                          ('all', 'Everything'),
                          ('installed', 'Installed'),
                          ('not_installed', 'Not installed'),
                          ('attention', 'Needs attention'),
                        ])
                          ChoiceChip(
                            key: Key('state-$value'),
                            label: Text(label),
                            selected: state == value,
                            onSelected: (_) => setState(() => state = value),
                          ),
                      ],
                    ),
                    const SizedBox(height: 16),
                    if (list.isEmpty)
                      const Padding(
                        padding: EdgeInsets.all(32),
                        child: Center(
                          child: Text(
                            'No packs match. Try another name or filter.',
                            style: TextStyle(color: muted),
                          ),
                        ),
                      ),
                  ],
                ),
              ),
              SliverPadding(
                padding: const EdgeInsets.fromLTRB(24, 0, 24, 32),
                sliver: SliverGrid.builder(
                  gridDelegate: SliverGridDelegateWithFixedCrossAxisCount(
                    crossAxisCount: columns,
                    mainAxisSpacing: 16,
                    crossAxisSpacing: 16,
                    // Tall enough for two-line names at larger text sizes.
                    mainAxisExtent:
                        MediaQuery.textScalerOf(context).scale(1) * 130 + 40,
                  ),
                  itemCount: list.length,
                  itemBuilder: (context, i) {
                    final p = list[i];
                    final id = p['id'] as String;
                    return PackCard(
                      key: Key('pack-$id'),
                      pack: p,
                      onIssues: needsAttention(p) ? () => issues(p) : null,
                    );
                  },
                ),
              ),
            ],
          );
        },
      ),
    );
  }
}

class _Stat extends StatelessWidget {
  const _Stat({
    super.key,
    required this.icon,
    required this.color,
    required this.text,
  });
  final IconData icon;
  final Color color;
  final String text;
  @override
  Widget build(BuildContext context) => Container(
    padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
    decoration: BoxDecoration(
      color: surface,
      borderRadius: BorderRadius.circular(20),
      border: Border.all(color: line),
    ),
    child: Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        Icon(icon, size: 16, color: color),
        const SizedBox(width: 6),
        Text(text, style: const TextStyle(color: ink, fontSize: 13)),
      ],
    ),
  );
}

/// One pack: its kind, name, and what is on this computer.
class PackCard extends StatelessWidget {
  const PackCard({super.key, required this.pack, this.onIssues});
  final Map<String, dynamic> pack;
  final VoidCallback? onIssues;

  @override
  Widget build(BuildContext context) {
    final kind = pack['kind']?.toString() ?? 'kit';
    final color = kindColors[kind] ?? muted;
    final status = pack['status']?.toString() ?? '';
    final (IconData icon, Color tone, String label) = switch (status) {
      'installed' => (Icons.check_circle_outline_rounded, teal, 'Installed'),
      'incomplete' => (Icons.error_outline_rounded, attention, 'Incomplete'),
      'not_installed' => (Icons.radio_button_unchecked, muted, 'Not installed'),
      _ => (Icons.cloud_outlined, muted, 'Available'),
    };
    final size = (pack['installed_bytes'] as num? ?? 0) > 0
        ? formatBytes(pack['installed_bytes'])
        : '';
    return Container(
      clipBehavior: Clip.antiAlias,
      decoration: BoxDecoration(
        color: surface,
        borderRadius: BorderRadius.circular(16),
        border: Border.all(color: line),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Container(height: 6, color: color),
          Expanded(
            child: Padding(
              padding: const EdgeInsets.fromLTRB(16, 8, 8, 16),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    '${kindNames[kind] ?? kind} - ${pack['id']}'.toUpperCase(),
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: TextStyle(
                      color: color,
                      fontSize: 10,
                      fontWeight: FontWeight.w700,
                      letterSpacing: 1,
                    ),
                  ),
                  const SizedBox(height: 4),
                  Flexible(
                    child: Text(
                      pack['name']?.toString() ?? '',
                      maxLines: 2,
                      overflow: TextOverflow.ellipsis,
                      style: const TextStyle(
                        color: ink,
                        fontSize: 17,
                        fontWeight: FontWeight.w600,
                        height: 1.2,
                      ),
                    ),
                  ),
                  const Spacer(),
                  Row(
                    children: [
                      Icon(icon, size: 16, color: tone),
                      const SizedBox(width: 4),
                      Flexible(
                        child: Text(
                          label,
                          style: TextStyle(color: tone, fontSize: 13),
                        ),
                      ),
                      if (onIssues != null)
                        TextButton(
                          key: Key('issues-${pack['id']}'),
                          onPressed: onIssues,
                          style: TextButton.styleFrom(
                            visualDensity: VisualDensity.compact,
                            tapTargetSize: MaterialTapTargetSize.shrinkWrap,
                            padding: const EdgeInsets.symmetric(horizontal: 8),
                            minimumSize: Size.zero,
                            foregroundColor: tone,
                          ),
                          child: const Text('Why?'),
                        ),
                      const SizedBox(width: 8),
                      Expanded(
                        child: Text(
                          size,
                          textAlign: TextAlign.end,
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: const TextStyle(color: muted, fontSize: 12),
                        ),
                      ),
                    ],
                  ),
                ],
              ),
            ),
          ),
        ],
      ),
    );
  }
}
