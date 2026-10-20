-- difftest: top=decl_only_arch std=2008 exit=2 ref=hand
-- ref=hand: the reference simulator stops in its elaboration step; the expected output is the
-- run-time failure of the constant's initial value.
-- An architecture without processes still elaborates its declarative part: a constant whose
-- initial value fails stops the run (LRM 14.4.1, 14.5).
entity decl_only_arch is
  generic (n : integer := 3);
end entity;

architecture a of decl_only_arch is
  constant c : natural := n - 5;
begin
end architecture;
