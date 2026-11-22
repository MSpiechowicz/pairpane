import { render, screen } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';

import { entry } from '../../../tests/panes';
import PaneRow from './PaneRow.svelte';

const row = (mark?: 'onlyHere' | 'newer' | 'older' | 'differs') =>
  render(PaneRow, { id: 'r-1', index: 1, entry: entry('a.txt'), cursor: false, selected: false, size: '0 B', mark });

describe('PaneRow marks', () => {
  it('tells a mark in words, in a title and as the description of the row', () => {
    row('newer');
    const option = screen.getByRole('option');

    expect(option).toHaveAttribute('title', 'Newer than in the other pane');
    expect(option).toHaveAccessibleDescription('Newer than in the other pane');
    expect(option).toHaveClass('mark-newer');
  });

  it('keeps the mark out of the name of the row: it is its description', () => {
    row('onlyHere');

    expect(screen.queryByRole('option', { name: /Only in this pane/ })).not.toBeInTheDocument();
    expect(screen.getByRole('option')).toHaveAccessibleDescription('Only in this pane');
  });

  it('has no mark text without a mark', () => {
    row();
    const option = screen.getByRole('option');

    expect(option).not.toHaveAttribute('title');
    expect(option).not.toHaveAttribute('aria-describedby');
    expect(option).not.toHaveClass('mark-newer', 'mark-onlyHere');
  });
});
