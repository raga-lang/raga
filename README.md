# Raga

A design of the Raga programming language, built around one consistent philosophy: every construct is the same binding shape.

**This repository is a language design, not an implementation.** There is no compiler or interpreter here. The design is written
as templates in Raga syntax so that it can be read, checked for consistency and discussed. When a template says what a line does
or prints, that is the intended meaning of the language, not the output of a program that was run.

- `hello.raga` is the quick start, read it top to bottom.
- `templates/` explains each area in full, in the order to read them:
  `01-basics`, `02-data-structures`, `03-control-flow`, `04-functions`, `05-structs-enums-unions`, `06-errors`,
  `07-classes-and-traits`, `08-memory`, `09-attributes-and-reflection`, `10-modules`, `11-standard-library`.

Editor support is in `../vs-code-extension`.
