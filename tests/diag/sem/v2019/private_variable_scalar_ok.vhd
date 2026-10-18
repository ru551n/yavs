-- rsim-test: std=2019
-- expect: none
-- private variables may be of any type (docs/design/lrm-checks.md L15; NVC accepts)
-- LRM 2019 5.6.2
package p is
  type outer is protected
    private variable iv : integer;
    procedure go;
  end protected;
end package;
