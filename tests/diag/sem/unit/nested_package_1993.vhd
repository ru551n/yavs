-- yavs-test: std=1993
-- expect: E1007 6:11 a package declaration inside a declarative part is not allowed in VHDL-1993
-- LRM 1993 1.1.2, 2.2, 2.5, 2.6: a package is a library unit only (#814)
package outer is
  constant c : integer := 1;
  package inner is
    constant d : integer := 2;
  end package;
end package;
