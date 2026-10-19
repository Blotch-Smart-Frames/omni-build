//! BOM stores ("bill of materials"), the container an asset catalog
//! (`Assets.car`) is kept in. Apple doesn't document the format. This
//! follows what Xcode's actool writes, which the open-source readers
//! (bomutils, and Timac's notes on the CAR format) agree on.
//!
//! A store is a table of numbered blocks of bytes, some of them named
//! ("vars"), and trees whose keys and values are blocks too. The store's
//! own structures are big-endian. What the blocks hold is up to the file
//! that uses the store: an asset catalog's are little-endian.

use anyhow::{Context, Result, bail};

use crate::bytes::Reader;

const MAGIC: &[u8; 8] = b"BOMStore";
/// The header, which the first block follows.
const HEADER_SIZE: usize = 512;
/// actool starts each block, the vars and the index on a 16-byte boundary.
const ALIGNMENT: usize = 16;
/// After the free list's count, actool leaves room for two entries in it.
const FREE_LIST_SLOTS: usize = 2;
/// A tree block starts with this, and its root's block number follows.
const TREE: &[u8; 4] = b"tree";

#[derive(Debug, PartialEq, Eq)]
pub struct Bom {
    /// The header's block count, kept as it was read: blocks are only ever
    /// replaced, so it stays true.
    block_count: u32,
    /// Each block's contents, by number. Block 0 is the null block, and a
    /// number no block uses is empty.
    blocks: Vec<Vec<u8>>,
    /// The named blocks, in the store's order.
    vars: Vec<(Vec<u8>, u32)>,
}

impl Bom {
    pub fn parse(data: &[u8]) -> Result<Self> {
        if !data.starts_with(MAGIC) {
            bail!("it isn't a BOM store");
        }
        let mut header = Reader::at(data, MAGIC.len());
        header.skip(4); // The version, 1.
        let block_count = header.u32_be();
        let index_at = header.u32_be();
        header.skip(4); // The index's length.
        let vars_at = header.u32_be();

        // The index is the block table: how many entries, then each block's
        // address and length.
        let mut index = Reader::at(data, index_at as usize);
        let entries = index.u32_be() as usize;
        let table = index.bytes(entries.saturating_mul(8));
        let blocks = table
            .as_chunks::<8>()
            .0
            .iter()
            .map(|entry| {
                let mut entry = Reader::at(entry, 0);
                let (address, length) = (entry.u32_be() as usize, entry.u32_be() as usize);
                data.get(address..address.saturating_add(length))
                    .map(<[u8]>::to_vec)
            })
            .collect::<Option<Vec<_>>>();

        let mut vars_reader = Reader::at(data, vars_at as usize);
        let vars = (0..vars_reader.u32_be())
            .map_while(|_| {
                let block = vars_reader.u32_be();
                let length = usize::from(vars_reader.u8());
                let name = vars_reader.bytes(length).to_vec();
                // A count past the end stops at the end.
                (!vars_reader.overrun()).then_some((name, block))
            })
            .collect();

        match blocks {
            Some(blocks) if !(header.overrun() || index.overrun() || vars_reader.overrun()) => {
                Ok(Self {
                    block_count,
                    blocks,
                    vars,
                })
            }
            _ => bail!("it's truncated"),
        }
    }

    /// A block's contents. A number past the table is an empty block, which
    /// whatever expected more of it finds truncated.
    pub fn block(&self, number: u32) -> &[u8] {
        self.blocks.get(number as usize).map_or(&[], Vec::as_slice)
    }

    /// Replaces the contents of a block already in the table.
    pub fn set_block(&mut self, number: u32, contents: Vec<u8>) {
        self.blocks[number as usize] = contents;
    }

    /// The number of the block named `name`.
    pub fn var(&self, name: &str) -> Option<u32> {
        self.vars
            .iter()
            .find(|(var, _)| var == name.as_bytes())
            .map(|&(_, block)| block)
    }

    /// The (key, value) block numbers in the tree named `name`, in the
    /// tree's order.
    pub fn tree(&self, name: &str) -> Result<Vec<(u32, u32)>> {
        let tree = self
            .var(name)
            .with_context(|| format!("it has no {name}"))?;
        let mut tree = Reader::at(self.block(tree), 0);
        if tree.bytes(TREE.len()) != TREE {
            bail!("its {name} isn't a tree");
        }
        tree.skip(4); // The version, 1.
        // Each node ("paths") is a block: whether it's a leaf, how many
        // entries it has, links to its neighbours, which the walk down the
        // tree doesn't need, then each entry's value and key. A branch's
        // values are its children.
        let mut nodes = vec![tree.u32_be()];
        let mut entries = Vec::new();
        let mut visited = 0;
        while let Some(node) = nodes.pop() {
            visited += 1;
            if visited > self.blocks.len() {
                bail!("its {name} tree is a loop");
            }
            let mut node = Reader::at(self.block(node), 0);
            let leaf = node.u16_be() == 1;
            let count = node.u16_be();
            node.skip(8);
            let mut children = Vec::new();
            for _ in 0..count {
                let (value, key) = (node.u32_be(), node.u32_be());
                if leaf {
                    entries.push((key, value));
                } else {
                    children.push(value);
                }
            }
            if node.overrun() {
                bail!("its {name} tree is truncated");
            }
            // Last in, first out: the leftmost child is walked first.
            nodes.extend(children.into_iter().rev());
        }
        Ok(entries)
    }

    /// The store as actool lays it out: the header, the blocks in order,
    /// the vars, then the index. An unchanged store is written back byte
    /// for byte.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = vec![0; HEADER_SIZE];
        let mut table = Vec::new();
        for block in &self.blocks {
            if block.is_empty() {
                table.push((0, 0));
            } else {
                pad(&mut out);
                table.push((u32_of(out.len()), u32_of(block.len())));
                out.extend(block);
            }
        }

        pad(&mut out);
        let vars_at = out.len();
        out.extend(u32_of(self.vars.len()).to_be_bytes());
        for (name, block) in &self.vars {
            out.extend(block.to_be_bytes());
            out.push(u8::try_from(name.len()).expect("a var's name was read with a u8 length"));
            out.extend(name);
        }
        let vars_len = out.len() - vars_at;

        pad(&mut out);
        let index_at = out.len();
        out.extend(u32_of(table.len()).to_be_bytes());
        for (address, length) in table {
            out.extend(address.to_be_bytes());
            out.extend(length.to_be_bytes());
        }
        // The free list is empty: the blocks are packed.
        out.extend(0u32.to_be_bytes());
        out.extend([0; FREE_LIST_SLOTS * 8]);
        let index_len = out.len() - index_at;

        let mut header = MAGIC.to_vec();
        for field in [
            1,
            self.block_count,
            u32_of(index_at),
            u32_of(index_len),
            u32_of(vars_at),
            u32_of(vars_len),
        ] {
            header.extend(field.to_be_bytes());
        }
        out[..header.len()].copy_from_slice(&header);
        out
    }
}

/// Pads `out` to the next block boundary.
fn pad(out: &mut Vec<u8>) {
    out.resize(out.len().next_multiple_of(ALIGNMENT), 0);
}

fn u32_of(value: usize) -> u32 {
    u32::try_from(value).expect("BOM stores are under 4 GiB")
}

#[cfg(test)]
pub mod tests {
    use super::Bom;
    use crate::ios::tests::shell_file;

    /// The shell's asset catalog, as actool wrote it.
    pub fn shell_catalog() -> Vec<u8> {
        shell_file("OmniBuildShell.app/Assets.car")
    }

    /// A store of these blocks (block 0, the null block, comes first) and
    /// vars.
    pub fn store(blocks: &[&[u8]], vars: &[(&str, u32)]) -> Bom {
        let count = u32::try_from(blocks.len()).unwrap();
        Bom {
            block_count: count,
            blocks: [&[][..]]
                .iter()
                .chain(blocks)
                .map(|block| block.to_vec())
                .collect(),
            vars: vars
                .iter()
                .map(|&(name, block)| (name.as_bytes().to_vec(), block))
                .collect(),
        }
    }

    /// A tree block whose root is block `root`.
    pub fn tree(root: u32) -> Vec<u8> {
        let mut tree = b"tree".to_vec();
        for field in [1, root, 4096, 1] {
            tree.extend(u32::to_be_bytes(field));
        }
        tree.push(0);
        tree
    }

    /// A tree node: a leaf's entries are (value, key) block numbers, and a
    /// branch's are (child, key).
    pub fn node(leaf: bool, entries: &[(u32, u32)]) -> Vec<u8> {
        let mut node = u16::from(leaf).to_be_bytes().to_vec();
        node.extend(u16::try_from(entries.len()).unwrap().to_be_bytes());
        node.extend([0; 8]);
        for &(value, key) in entries {
            node.extend(value.to_be_bytes());
            node.extend(key.to_be_bytes());
        }
        node
    }

    #[test]
    fn writes_an_unchanged_catalog_back_byte_for_byte() {
        let catalog = shell_catalog();

        let bom = Bom::parse(&catalog).unwrap();

        assert_eq!(bom.to_bytes(), catalog);
        assert_eq!(bom.var("CARHEADER"), Some(1));
        assert_eq!(&bom.block(1)[..4], b"RATC");
        assert_eq!(bom.var("NONE"), None);
    }

    #[test]
    fn walks_a_tree_from_its_root_down_to_each_leaf() {
        // Block 1 is the tree, 2 its root (a branch), 3 and 4 its leaves,
        // and 5 to 8 the leaves' keys and values.
        let bom = store(
            &[
                &tree(2),
                &node(false, &[(3, 5), (4, 7)]),
                &node(true, &[(6, 5)]),
                &node(true, &[(8, 7)]),
                b"key a",
                b"value a",
                b"key b",
                b"value b",
            ],
            &[("TREE", 1)],
        );

        let entries = bom.tree("TREE").unwrap();

        assert_eq!(entries, [(5, 6), (7, 8)]);
        assert_eq!(bom.block(entries[1].1), b"value b");
    }

    #[test]
    fn replaces_a_block_and_moves_the_ones_after_it() {
        let mut bom = store(&[b"one", b"two", b"three"], &[("TWO", 2)]);

        bom.set_block(2, b"a longer two".to_vec());

        let written = Bom::parse(&bom.to_bytes()).unwrap();
        assert_eq!(written, bom);
        assert_eq!(written.block(2), b"a longer two");
        assert_eq!(written.block(3), b"three");
        // Past the table.
        assert_eq!(written.block(9), b"");
    }

    #[test]
    fn reports_a_store_it_cant_read() {
        let catalog = shell_catalog();
        let header = |at: usize| u32::from_be_bytes(catalog[at..at + 4].try_into().unwrap());
        let (index, vars) = (header(16) as usize, header(24) as usize);
        // Block 1's length, after the index's count and block 0's entry.
        let mut long_block = catalog.clone();
        long_block[index + 16..index + 20].copy_from_slice(&u32::MAX.to_be_bytes());
        let mut more_vars = catalog.clone();
        more_vars[vars..vars + 4].copy_from_slice(&u32::MAX.to_be_bytes());
        let cases: [(&[u8], &str); 4] = [
            (b"not a BOM store", "it isn't a BOM store"),
            (&catalog[..600], "it's truncated"),
            (&long_block, "it's truncated"),
            (&more_vars, "it's truncated"),
        ];

        for (data, message) in cases {
            assert_eq!(Bom::parse(data).unwrap_err().to_string(), message);
        }
    }

    #[test]
    fn reports_a_tree_it_cant_walk() {
        let bom = store(
            &[
                &tree(2),
                &node(false, &[(2, 0)]),
                b"not a tree",
                &tree(5),
                &node(true, &[(1, 1)])[..14],
            ],
            &[("LOOP", 1), ("NOT", 3), ("SHORT", 4)],
        );
        let cases = [
            ("NONE", "it has no NONE"),
            ("NOT", "its NOT isn't a tree"),
            ("LOOP", "its LOOP tree is a loop"),
            ("SHORT", "its SHORT tree is truncated"),
        ];

        for (name, message) in cases {
            assert_eq!(bom.tree(name).unwrap_err().to_string(), message);
        }
    }
}
