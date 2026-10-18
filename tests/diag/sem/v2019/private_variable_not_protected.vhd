-- rsim-test: std=2019
-- expect: E1904 6:22 private variable `iv` must be of a protected type
-- LRM 2019 5.6.2
package p is
  type outer is protected
    private variable iv : integer;
    procedure go;
  end protected;
end package;
