# PC2

## 1.build tree-c

sc-02-data-layer-1.dat ## size 8388608 == 8<<20
sc-02-data-layer-2.dat ## size 8388608 == 8<<20


| layer  1 | node0 | node1 | ... | node end |
| -------- | ----- | ----- | --- | -------- |
| layer  N | node0 | node1 | ... | node end |

对每个layer的 node x 进行竖向hash
得到一个用于构建8叉merkle树的leaf节点顺序表


| tree-leaf | node0 | node1 | ... | node end |
| --------- | ----- | ----- | --- | -------- |

基于这些叶节点构建  oct-merkle-tree 最终文件 ( 16M 构建了2颗, 因为16M没法构建出全8叉树 )

sc-02-data-tree-c.dat  ## size == 9586976
== (8<<20)+(8<<17)+(8<<14)+(8<<11)+(8<<8)+(8<<5)+(8<<2)

## 2.build tree-r


| sealed-file | node0 | node1 | ... | node end |
| ----------- | ----- | ----- | --- | -------- |
| layer-last  | node0 | node1 | ... | node end |

竖向"+" sealed-file 和 最后一个layer 的每个节点, 这个"+"的实现在FFI中, 具体做了什么暂时不清楚.
相加得到add-res, 并且将这个结果写回sealed-file


| add-res | node0 | node1 | ... | node end |
| ------- | ----- | ----- | --- | -------- |

基于 add-res 构建一个除去了最底部3层节点 ( 但是debug时看见的配置是丢2层, 这中间还有疑点 )( 512M也是丢3层 )( 2K 参数丢一层, 实际丢2层 )( 破案, 一个是disk tree 一个是lc tree, lc tree本身就没算底层 ) 的8叉merkle树
sc-02-data-tree-r-last.dat ## size == 18720
==  (8<<11)+(8<<8)+(8<<5)+(8<<2)

512M  1198368 == (512<<11)+(512<<8)+(512<<5)+(512<<2)+(512>>1) + (512>>4)

2K 32=
