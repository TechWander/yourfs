#ifndef YFS_SCAN_H
#define YFS_SCAN_H

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <errno.h>
#include <fcntl.h>
#include <getopt.h>
#include <unistd.h>
#include <sys/types.h>
#include <dirent.h>
#include <sys/stat.h>
#include <math.h>
#include "./yfs_common.h"


typedef struct TreeNode {
    char name[256];
    int isDir;
    struct TreeNode *child;
    struct TreeNode *sibling;
} TreeNode;



TreeNode* createNode(char name[], int isDir) {
    TreeNode *node = (TreeNode*)malloc(sizeof(TreeNode));
    strcpy(node->name, name);
    node->isDir = isDir;
    node->child = NULL;
    node->sibling = NULL;
    return node;
}



TreeNode* createTree(char path[], int replace_num, char pre[], FILE *fp) {
    DIR *dir;
    struct dirent *ent;
    char childPath[4096], outputPath[4096];
    TreeNode *root = NULL, *node, *lastNode = NULL;
    char parent[4096],name[256];

    dir = opendir(path);
    if (dir != NULL) {

        while ((ent = readdir(dir)) != NULL) {

            if (strcmp(ent->d_name, ".") == 0 || strcmp(ent->d_name, "..") == 0) { continue; }
            sprintf(childPath, "%s/%s", path, ent->d_name);
            sprintf(outputPath, "%s/%s", path, ent->d_name);
            memmove(outputPath, outputPath + replace_num, strlen(outputPath));

            get_path(outputPath, parent, name);
            fprintf(fp,"%-25s  %-25s  %-25s\n",parent,name,outputPath);

            if (ent->d_type == DT_DIR) {

                node = createNode(ent->d_name, 1);
                if (strcmp(ent->d_name, ".") == 0 || strcmp(ent->d_name, "..") == 0) {

                } else {

                  node->child = createTree(childPath,replace_num,pre,fp);

                }
            } else {

                node = createNode(ent->d_name, 0);

            }

            if (root == NULL) {

                root = node;

            } else {

                lastNode->sibling = node;

            }

            lastNode = node;
        }

        closedir(dir);
    }
    return root;
}


void traverseTree(TreeNode *node, int depth) {
    if (node == NULL) {
        return;
    }
    for (int i = 0; i < depth; i++) {
        printf("  ");
    }

    printf("%s\n", node->name);

    if (node->isDir) {
        traverseTree(node->child, depth + 2);
    }
    traverseTree(node->sibling, depth);
}



#endif

