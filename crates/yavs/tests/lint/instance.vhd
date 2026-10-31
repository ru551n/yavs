-- lint_751 of an instance: the architecture named is not in the library.
entity leaf is
end entity;

architecture rtl of leaf is
begin
end architecture;

entity top is
end entity;

architecture sim of top is
begin
  good : entity work.leaf(rtl);
  bad : entity work.leaf(no_such); -- expect: lint_751
  plain : entity work.leaf;

  off : if false generate
    dead : entity work.leaf(also_no_such);
  end generate;
end architecture;
