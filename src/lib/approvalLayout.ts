// Layout helper for the approval document pane (ui.md §8 / design §3.5).
// Splits a page's concatenated text into segments so nested fields stay visible
// (innermost field owns the overlapping slice; parents still render around it).

import type { ApprovalPage, DetectedFieldDto, FieldDecisionKind } from './api'

export type HighlightKind = 'plain' | 'undecided' | 'keep_visible' | 'redact'

export interface TextSegment {
  text: string
  fieldId: string | null
  kind: HighlightKind
  nested: boolean
}

const encoder = new TextEncoder()
const decoder = new TextDecoder()

function byteToChar(page: ApprovalPage, absByte: number): number {
  let charIndex = 0
  for (const span of page.spans) {
    const spanBytes = encoder.encode(span.text)
    const spanEnd = span.byte_offset + spanBytes.length
    if (absByte <= span.byte_offset) {
      return charIndex
    }
    if (absByte <= spanEnd) {
      const local = Number(absByte - span.byte_offset)
      return charIndex + decoder.decode(spanBytes.slice(0, local)).length
    }
    charIndex += span.text.length
  }
  return charIndex
}

function mergeAdjacent(segments: TextSegment[]): TextSegment[] {
  const out: TextSegment[] = []
  for (const segment of segments) {
    const last = out[out.length - 1]
    if (
      last &&
      last.fieldId === segment.fieldId &&
      last.kind === segment.kind &&
      last.nested === segment.nested
    ) {
      last.text += segment.text
    } else {
      out.push({ ...segment })
    }
  }
  return out
}

export function layoutPage(
  page: ApprovalPage,
  fields: DetectedFieldDto[],
  decisions: Record<string, FieldDecisionKind>,
): TextSegment[] {
  const pageText = page.spans.map((span) => span.text).join('')
  if (pageText.length === 0) {
    return []
  }

  const ranges = fields
    .filter((field) => field.span.page_index === page.page_index)
    .map((field) => ({
      id: field.id,
      start: byteToChar(page, field.span.byte_offset),
      end: byteToChar(page, field.span.byte_offset + field.span.byte_length),
      parent: field.parent_field_id,
    }))
    .filter((range) => range.end > range.start)
    .map((range) => ({
      ...range,
      start: Math.max(0, Math.min(range.start, pageText.length)),
      end: Math.max(0, Math.min(range.end, pageText.length)),
    }))

  const cuts = new Set<number>([0, pageText.length])
  for (const range of ranges) {
    cuts.add(range.start)
    cuts.add(range.end)
  }
  const points = [...cuts].sort((a, b) => a - b)

  const segments: TextSegment[] = []
  for (let i = 0; i < points.length - 1; i++) {
    const from = points[i]
    const to = points[i + 1]
    if (from >= to) continue
    const covering = ranges
      .filter((range) => range.start <= from && range.end >= to)
      .sort((a, b) => a.end - a.start - (b.end - b.start))
    const innermost = covering[0]
    if (!innermost) {
      segments.push({ text: pageText.slice(from, to), fieldId: null, kind: 'plain', nested: false })
      continue
    }
    const decision = decisions[innermost.id]
    const kind: HighlightKind =
      decision === 'keep_visible' || decision === 'redact' ? decision : 'undecided'
    segments.push({
      text: pageText.slice(from, to),
      fieldId: innermost.id,
      kind,
      nested: covering.length > 1 || innermost.parent !== null,
    })
  }
  return mergeAdjacent(segments)
}
