# Example textures (placeholders)

`stone.png`, `dirt.png` and `grass_top.png` are 16 x 16 placeholders made by
Claude so Andrew can see what a texture looks like on a block. They are not
art direction, and no block uses them unless Andrew picks one: in the game,
press F1, open Blocks, click a block's name, and click an example under
*Texture*. Painting on an example makes the block its own copy, so these
files never change.

Each one averages to its block's colour in `blocks.json`, so from far away a
block wearing one looks the same as before. They were made by
`game/tools/make_example_textures.py`; delete or replace them freely.
