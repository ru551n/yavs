-- lint_751: a configuration names an architecture that is not declared.
entity dff is
end entity;

architecture rtl of dff is
begin
end architecture;

entity tb is
end entity;

architecture sim of tb is
  component dff is
  end component;
begin
  i_good : dff;
  i_bad : dff;
  i_open : dff;
end architecture;

configuration cfg of tb is
  for sim
    for i_good : dff
      use entity work.dff(rtl);
    end for;
    for i_bad : dff
      use entity work.dff(no_such_arch); -- expect: lint_751
    end for;
    for others : dff
      use entity work.dff;
    end for;
  end for;
end configuration;
