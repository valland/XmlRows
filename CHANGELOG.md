# Changelog

Notable user-facing changes to XmlRows are documented here. The project uses
[Semantic Versioning](https://semver.org/), with the usual flexibility for
versions below 1.0.0.

## Unreleased

## 0.1.3 - 2026-09-21

### Added

- `Select highlighted XML` in the source toolbar turns one row or a contiguous
  range of highlighted table rows into a real editor selection.
- Clicking `#` selects or clears all displayed table rows.

### Fixed

- Formatting XML now preserves the selected table and keeps the table controls
  available.

## 0.1.2 - 2026-09-21

### Added

- Multi-row selection with Shift-click and Cmd/Ctrl-click.
- Simultaneous XML source highlighting for all selected rows.

### Changed

- Copy uses selected rows when a row selection exists.
- Clicking outside the table clears row and XML source highlights.
- Table cells remain a single-cell selection model; browser text selection
  across cells is disabled.

### Fixed

- Row-number highlights now match selected cells on alternating row colors.
- Editing a cell no longer changes its dimensions or surrounding column widths.
- A cell keeps the same background and border while it is being edited.

## 0.1.1 - 2026-09-21

### Changed

- Standardized the product name and generated application bundles as XmlRows.

### Fixed

- Restored window dragging from the custom application toolbar.

## 0.1.0 - 2026-09-20

### Added

- Initial macOS desktop application with linked XML source, structure and table
  views.
- In-place cell editing, sorting, spreadsheet copy and row duplication.
- Rust command-line interface for inspecting and exporting XML data.
