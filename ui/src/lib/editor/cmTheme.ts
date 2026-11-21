import { HighlightStyle, syntaxHighlighting } from '@codemirror/language';
import { EditorView } from '@codemirror/view';
import { tags } from '@lezer/highlight';

/**
 * The editor's look, made of the app's colour variables (lib/design/tokens.css), so a change of theme restyles the
 * open editor without making it again.
 */
const chrome = EditorView.theme({
  '&': {
    height: '100%',
    backgroundColor: 'var(--bg)',
    color: 'var(--text)',
    fontSize: '12.5px',
  },
  '&.cm-focused': { outline: 'none' },
  '.cm-scroller': { fontFamily: 'var(--font-mono)', lineHeight: '1.5' },
  '.cm-content': { caretColor: 'var(--text)' },
  '.cm-cursor, .cm-dropCursor': { borderLeftColor: 'var(--text)' },
  '.cm-activeLine': { backgroundColor: 'color-mix(in srgb, var(--accent) 8%, transparent)' },
  '&.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground, .cm-selectionBackground': {
    backgroundColor: 'color-mix(in srgb, var(--accent) 30%, transparent)',
  },
  '.cm-gutters': {
    backgroundColor: 'var(--panel)',
    color: 'var(--dim)',
    border: 'none',
    borderRight: '1px solid var(--line)',
  },
  '.cm-activeLineGutter': { backgroundColor: 'var(--panel-2)', color: 'var(--text)' },
  '.cm-searchMatch': {
    backgroundColor: 'color-mix(in srgb, var(--sel) 30%, transparent)',
    outline: '1px solid color-mix(in srgb, var(--sel) 60%, transparent)',
  },
  '.cm-searchMatch.cm-searchMatch-selected': { backgroundColor: 'color-mix(in srgb, var(--sel) 55%, transparent)' },
  '.cm-panels': { backgroundColor: 'var(--panel)', color: 'var(--text)' },
  '.cm-panels.cm-panels-top': { borderBottom: '1px solid var(--line)' },
  '.cm-panel.cm-search': { fontFamily: 'var(--font-ui)' },
  '.cm-panel.cm-search label': { color: 'var(--dim)' },
  '.cm-textfield': {
    backgroundColor: 'var(--panel-2)',
    color: 'var(--text)',
    border: '1px solid var(--line)',
    borderRadius: '6px',
  },
  '.cm-button': {
    backgroundColor: 'var(--panel-2)',
    backgroundImage: 'none',
    color: 'var(--text)',
    border: '1px solid var(--line)',
    borderRadius: '6px',
  },
  '.cm-panel.cm-search [name=close]': { color: 'var(--dim)' },
});

const highlight = HighlightStyle.define([
  { tag: [tags.keyword, tags.modifier, tags.operatorKeyword, tags.controlKeyword], color: 'var(--accent-2)' },
  { tag: [tags.string, tags.special(tags.string), tags.regexp], color: 'var(--ok)' },
  { tag: [tags.number, tags.bool, tags.null, tags.atom, tags.literal], color: 'var(--sel)' },
  { tag: [tags.comment, tags.docComment], color: 'var(--dim)', fontStyle: 'italic' },
  { tag: [tags.typeName, tags.className, tags.namespace], color: 'var(--dir)' },
  { tag: [tags.function(tags.variableName), tags.function(tags.propertyName), tags.propertyName], color: 'var(--accent)' },
  { tag: [tags.heading], color: 'var(--accent-2)', fontWeight: '600' },
  { tag: [tags.link, tags.url], color: 'var(--accent)', textDecoration: 'underline' },
  { tag: tags.emphasis, fontStyle: 'italic' },
  { tag: tags.strong, fontWeight: '700' },
  { tag: tags.invalid, color: 'var(--danger)' },
]);

export const theme = [chrome, syntaxHighlighting(highlight)];
