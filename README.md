# Dev note

## TCP
- TCP segment size is somewhat unpredictable due to several factors. Ref: https://datatracker.ietf.org/doc/html/rfc9293#name-segmentation. This behavior need to be considered when implement tcp listener.