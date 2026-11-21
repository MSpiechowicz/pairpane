import { fireEvent, render, screen, within } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';

import type { MdBlock } from '$lib/ipc/bindings/MdBlock';
import type { MdDocument } from '$lib/ipc/bindings/MdDocument';
import type { MdInline } from '$lib/ipc/bindings/MdInline';

import { FakeFile, openedSession } from '../../../tests/view';
import MarkdownView from './MarkdownView.svelte';

const text = (value: string): MdInline => ({ kind: 'text', text: value });
const paragraph = (...inlines: MdInline[]): MdBlock => ({ kind: 'paragraph', inlines });

async function open(doc: MdDocument | (() => Promise<MdDocument>)) {
  const world = await openedSession(new FakeFile('# x'), () => ({ kind: 'markdown' }), { view_markdown: () => (typeof doc === 'function' ? doc() : doc) });
  const view = render(MarkdownView, { session: world.session });

  return { ...world, view };
}

describe('MarkdownView', () => {
  it('asks for the document in the session`s encoding', async () => {
    const { sent } = await open({ blocks: [paragraph(text('hello'))], truncated: false });

    expect(await screen.findByText('hello')).toBeInTheDocument();
    expect(sent('view_markdown')).toEqual([{ session: 1, encoding: 'UTF-8' }]);
  });

  it('asks again, and shows the new answer, when the encoding changes', async () => {
    let calls = 0;
    const { session, sent } = await open({ blocks: [], truncated: false });
    session.ipc.invoke = vi.fn(async () => ({ blocks: [paragraph(text(`answer ${++calls}`))], truncated: false })) as never;
    await vi.waitFor(() => expect(sent('view_markdown')).toHaveLength(1));

    session.encoding = 'windows-1250';

    expect(await screen.findByText('answer 1')).toBeInTheDocument();
    expect(vi.mocked(session.ipc.invoke)).toHaveBeenCalledWith('view_markdown', { session: 1, encoding: 'windows-1250' });
  });

  it('draws headings, lists, task boxes, quotes, tables, code and rules as elements', async () => {
    const { view } = await open({
      truncated: false,
      blocks: [
        { kind: 'heading', level: 2, inlines: [text('Title')] },
        {
          kind: 'list',
          ordered: true,
          start: 3,
          items: [
            { checked: null, blocks: [paragraph(text('first'))] },
            { checked: true, blocks: [paragraph(text('done'))] },
            { checked: false, blocks: [paragraph(text('todo'))] },
          ],
        },
        { kind: 'quote', blocks: [paragraph({ kind: 'strong', children: [text('bold')] }, { kind: 'break' }, { kind: 'code', text: 'x < y' })] },
        {
          kind: 'table',
          header: [[text('Name')], [text('Size')]],
          rows: [[[text('a')], [text('1')]]],
        },
        { kind: 'codeBlock', language: 'rust', text: 'fn main() {}\n' },
        { kind: 'rule' },
      ],
    });

    expect(await screen.findByRole('heading', { level: 2, name: 'Title' })).toBeInTheDocument();
    const list = view.container.querySelector('ol');
    expect(list).toHaveAttribute('start', '3');
    expect(within(list as HTMLElement).getAllByRole('listitem')).toHaveLength(3);
    const boxes = screen.getAllByRole('checkbox');
    expect(boxes.map((box) => (box as HTMLInputElement).checked)).toEqual([true, false]);
    expect(boxes.every((box) => (box as HTMLInputElement).disabled)).toBe(true);
    expect(view.container.querySelector('blockquote strong')).toHaveTextContent('bold');
    expect(view.container.querySelector('blockquote br')).not.toBeNull();
    expect(view.container.querySelector('blockquote code')).toHaveTextContent('x < y');
    expect(screen.getAllByRole('columnheader').map((cell) => cell.textContent)).toEqual(['Name', 'Size']);
    expect(screen.getAllByRole('cell').map((cell) => cell.textContent)).toEqual(['a', '1']);
    expect(view.container.querySelector('pre code')).toHaveTextContent('fn main() {}');
    expect(view.container.querySelector('hr')).not.toBeNull();
  });

  it('shows markup in the file as the text it is, and nothing in it navigates', async () => {
    const { view } = await open({
      truncated: false,
      blocks: [
        paragraph(text('<script>alert(1)</script> and <img src=x onerror=alert(2)>')),
        paragraph(
          { kind: 'link', href: 'https://example.com/a', children: [text('site')] },
          { kind: 'link', href: null, children: [text('dead')] },
          { kind: 'image', alt: 'a picture' },
        ),
      ],
    });

    expect(await screen.findByText(/<script>alert\(1\)<\/script>/)).toBeInTheDocument();
    expect(view.container.querySelector('script')).toBeNull();
    expect(view.container.querySelector('img')).toBeNull();
    expect(view.container.querySelectorAll('[href]')).toHaveLength(0);
    expect(screen.queryAllByRole('link')).toHaveLength(0);

    const link = screen.getByText('site');
    expect(link).toHaveAttribute('title', 'https://example.com/a');
    await fireEvent.click(link);
    expect(screen.getByText('dead')).not.toHaveAttribute('title');
    expect(screen.getByText('a picture')).toBeInTheDocument();
  });

  it('says when only the start of the file is shown', async () => {
    await open({ blocks: [paragraph(text('start'))], truncated: true });

    expect(await screen.findByText('Only the start of this file is shown')).toBeInTheDocument();
  });

  it('shows the backend`s words when the file cannot be read', async () => {
    await open(() => Promise.reject({ code: 'io', message: 'The file is too big to render' }));

    expect(await screen.findByRole('alert')).toHaveTextContent('The file is too big to render');
  });
});
