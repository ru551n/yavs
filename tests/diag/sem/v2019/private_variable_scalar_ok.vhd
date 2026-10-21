-- yavs-test: std=2019
-- expect: none
-- LRM 2019 5.6.2 (no rule found that a private variable must be protected; permissive, as sim1)
package p is
  type outer is protected
    private variable iv : integer;
    procedure go;
  end protected;
end package;
